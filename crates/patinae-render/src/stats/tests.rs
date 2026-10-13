use super::*;

#[test]
fn empty_mask_resolves_no_queries() {
    assert_eq!(written_query_pairs(0).count(), 0);
}

#[test]
fn each_mask_bit_resolves_only_its_timestamp_pair() {
    for pass in 0..TIMESTAMPS_PER_FRAME / 2 {
        let mut pairs = written_query_pairs(1 << pass);
        assert_eq!(pairs.next(), Some(pass * 2..pass * 2 + 2));
        assert_eq!(pairs.next(), None);
    }
}

#[test]
fn optional_passes_leave_gaps_in_resolved_queries() {
    let mask = (1 << Pass::Opaque as u32)
        | (1 << Pass::Ssao as u32)
        | (1 << Pass::Fxaa as u32)
        | (1 << Pass::FastOverlay as u32);
    assert_eq!(
        written_query_pairs(mask).collect::<Vec<_>>(),
        [6..8, 16..18, 22..24, 30..32]
    );
}

#[test]
fn all_masks_resolve_exactly_the_written_queries() {
    for mask in 0..1 << (TIMESTAMPS_PER_FRAME / 2) {
        let queries = written_query_pairs(mask).flatten().collect::<Vec<_>>();
        for query in 0..TIMESTAMPS_PER_FRAME {
            assert_eq!(queries.contains(&query), mask & (1 << (query / 2)) != 0);
        }
    }
}

fn wait_for_gpu(device: &wgpu::Device) {
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .expect("GPU work and readback must complete");
}

fn read_ticks(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u64> {
    buffer.slice(..).map_async(wgpu::MapMode::Read, |result| {
        result.expect("timestamp readback must map");
    });
    wait_for_gpu(device);
    let ticks = bytemuck::cast_slice(&buffer.slice(..).get_mapped_range()).to_vec();
    buffer.unmap();
    ticks
}

#[test]
#[ignore = "requires a GPU with TIMESTAMP_QUERY; run explicitly for timestamp validation"]
fn gpu_sparse_timestamp_resolve_preserves_layout_and_reuses_slots() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle().with_env();
    descriptor.flags |= wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("GPU adapter required; this test must not silently skip");
    eprintln!("Timestamp regression adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .expect("TIMESTAMP_QUERY support required");
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut collector = FrameStatsCollector::new(&device, &queue);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stats.test.target"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = target.create_view(&Default::default());

    // Independent reference: each pair has its own aligned resolve region.
    // Comparing exact ticks catches scratch-buffer overwrite or copy reordering.
    let reference_resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("stats.test.reference.resolve"),
        size: u64::from(TIMESTAMPS_PER_FRAME / 2) * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let reference_readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("stats.test.reference.readback"),
        size: u64::from(TIMESTAMPS_PER_FRAME) * u64::from(wgpu::QUERY_SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    // Nonzero stale durations expose an accidental resolve/copy of skipped pairs.
    let stale: Vec<u64> = (0..TIMESTAMPS_PER_FRAME)
        .map(|i| u64::from(i) + 1)
        .collect();
    let mut previous = vec![stale.clone(); IN_FLIGHT_FRAMES];
    for slot in &collector.slots {
        queue.write_buffer(
            &slot.lock().unwrap().readback_buffer,
            0,
            bytemuck::cast_slice(&stale),
        );
    }

    collector.frame_begin();
    // Start empty, then populate optional passes, skip them on reuse, and empty
    // the same slots again. ComputeBuild (queries 0/1) is never written, as in #31.
    for passes in [
        &[][..],
        &[Pass::FastOverlay, Pass::Fxaa, Pass::Ssao, Pass::Opaque][..],
        &[Pass::Opaque][..],
        &[][..],
    ] {
        for expected_slot in 0..IN_FLIGHT_FRAMES {
            let slot_index = collector.current_slot_idx();
            assert_eq!(slot_index, expected_slot);
            assert!(collector.recording);
            assert_eq!(collector.slots[slot_index].lock().unwrap().written_mask, 0);
            let frame_index = collector.frame_counter;
            collector.mark_record_start();
            let mut encoder = device.create_command_encoder(&Default::default());
            for &pass in passes {
                if matches!(pass, Pass::Ssao) {
                    let _pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        timestamp_writes: collector.compute_pass_timestamp_writes(pass),
                        ..Default::default()
                    });
                } else {
                    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        timestamp_writes: collector.render_pass_timestamp_writes(pass),
                        ..Default::default()
                    });
                }
            }
            for &pass in passes {
                let query = pass as u32 * 2;
                let offset = u64::from(pass as u32) * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
                encoder.resolve_query_set(
                    &collector.query_sets[slot_index],
                    query..query + 2,
                    &reference_resolve,
                    offset,
                );
                encoder.copy_buffer_to_buffer(
                    &reference_resolve,
                    offset,
                    &reference_readback,
                    u64::from(query) * u64::from(wgpu::QUERY_SIZE),
                    2 * u64::from(wgpu::QUERY_SIZE),
                );
            }
            collector.mark_record_end();
            collector.resolve_and_map(&mut encoder, &queue);
            assert_eq!(
                collector.slots[slot_index]
                    .lock()
                    .unwrap()
                    .state
                    .load(Ordering::Acquire),
                SLOT_NEEDS_MAP
            );
            assert!(collector.take_latest().is_none());
            // Submission must succeed before frame_begin requests any mapping.
            queue.submit([encoder.finish()]);
            collector.frame_begin();
            wait_for_gpu(&device);
            assert_eq!(
                collector.slots[slot_index]
                    .lock()
                    .unwrap()
                    .state
                    .load(Ordering::Acquire),
                SLOT_IDLE
            );
            let stats = collector
                .take_latest()
                .expect("frame stats must be published");
            assert_eq!(stats.frame_index, frame_index);
            assert!(stats.cpu_record_ms >= 0.0);
            assert_eq!(stats.gpu_compute_build_ms, -1.0);
            assert_eq!(stats.gpu_silhouette_ms, -1.0);

            let expected = read_ticks(&device, &reference_readback);
            for &pass in passes {
                let first = pass as usize * 2;
                previous[slot_index][first..first + 2].copy_from_slice(&expected[first..first + 2]);
            }
            let readback = collector.slots[slot_index]
                .lock()
                .unwrap()
                .readback_buffer
                .clone();
            assert_eq!(read_ticks(&device, &readback), previous[slot_index]);
            let mut total = 0.0;
            for (pass, actual) in [
                (Pass::Opaque, stats.gpu_opaque_ms),
                (Pass::Ssao, stats.gpu_ssao_ms),
                (Pass::Fxaa, stats.gpu_fxaa_ms),
                (Pass::FastOverlay, stats.gpu_fast_overlay_ms),
            ] {
                if passes.iter().any(|p| *p as u32 == pass as u32) {
                    let first = pass as usize * 2;
                    let elapsed = expected[first + 1].saturating_sub(expected[first]) as f32
                        * collector.ticks_to_ns
                        / 1_000_000.0;
                    assert_eq!(actual, elapsed);
                    total += elapsed;
                } else {
                    assert_eq!(actual, -1.0);
                }
            }
            // Summation order differs from the collector's order.
            assert!((stats.gpu_total_ms - total).abs() <= f32::EPSILON * total.max(1.0));
        }
    }
    assert!(pollster::block_on(validation.pop()).is_none());
}
