class T {
  static __wrap(e) {
    e = e >>> 0;
    const t = Object.create(T.prototype);
    return t.__wbg_ptr = e, U.register(t, t.__wbg_ptr, t), t;
  }
  __destroy_into_raw() {
    const e = this.__wbg_ptr;
    return this.__wbg_ptr = 0, U.unregister(this), e;
  }
  free() {
    const e = this.__destroy_into_raw();
    c.__wbg_webviewer_free(e, 0);
  }
  /**
   * @param {string} selection
   * @returns {number}
   */
  count_atoms(e) {
    try {
      const a = c.__wbindgen_add_to_stack_pointer(-16), s = l(e, c.__wbindgen_export, c.__wbindgen_export2), u = g;
      c.webviewer_count_atoms(a, this.__wbg_ptr, s, u);
      var t = w().getInt32(a + 0, !0), _ = w().getInt32(a + 4, !0), r = w().getInt32(a + 8, !0);
      if (r)
        throw d(_);
      return t >>> 0;
    } finally {
      c.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Create a new WebViewer bound to a `<canvas>` element.
   *
   * `picking_enabled = false` (default) drops hit-test picking readbacks
   * and the half-res picking target. Selection overlay is controlled
   * separately; silhouettes remain command/settings-driven.
   * @param {string} canvas_id
   * @param {boolean} picking_enabled
   * @param {boolean} selection_overlay_enabled
   * @returns {Promise<WebViewer>}
   */
  static create(e, t, _) {
    const r = l(e, c.__wbindgen_export, c.__wbindgen_export2), a = g, s = c.webviewer_create(r, a, t, _);
    return d(s);
  }
  /**
   * Create a WebViewer with an explicit renderer memory profile.
   *
   * Pass `"auto"` or an empty string to use adapter-based selection.
   * Accepted forced profiles are `"performance"`, `"balanced"`, and `"lite"`.
   * @param {string} canvas_id
   * @param {boolean} picking_enabled
   * @param {boolean} selection_overlay_enabled
   * @param {string} memory_profile
   * @returns {Promise<WebViewer>}
   */
  static createWithMemoryProfile(e, t, _, r) {
    const a = l(e, c.__wbindgen_export, c.__wbindgen_export2), s = g, u = l(r, c.__wbindgen_export, c.__wbindgen_export2), p = g, W = c.webviewer_createWithMemoryProfile(a, s, t, _, u, p);
    return d(W);
  }
  /**
   * Execute a command string. Returns JSON with output messages.
   * @param {string} command
   * @returns {any}
   */
  execute(e) {
    const t = l(e, c.__wbindgen_export, c.__wbindgen_export2), _ = g, r = c.webviewer_execute(this.__wbg_ptr, t, _);
    return d(r);
  }
  /**
   * @param {string} name
   * @returns {any}
   */
  get_label_object(e) {
    const t = l(e, c.__wbindgen_export, c.__wbindgen_export2), _ = g, r = c.webviewer_get_label_object(this.__wbg_ptr, t, _);
    return d(r);
  }
  /**
   * @returns {any}
   */
  get_labels() {
    const e = c.webviewer_get_labels(this.__wbg_ptr);
    return d(e);
  }
  /**
   * @returns {any}
   */
  get_movie_state() {
    const e = c.webviewer_get_movie_state(this.__wbg_ptr);
    return d(e);
  }
  /**
   * @param {string} name
   * @returns {any}
   */
  get_object_info(e) {
    const t = l(e, c.__wbindgen_export, c.__wbindgen_export2), _ = g, r = c.webviewer_get_object_info(this.__wbg_ptr, t, _);
    return d(r);
  }
  /**
   * @returns {any}
   */
  get_object_infos() {
    const e = c.webviewer_get_object_infos(this.__wbg_ptr);
    return d(e);
  }
  /**
   * @returns {any}
   */
  get_object_names() {
    const e = c.webviewer_get_object_names(this.__wbg_ptr);
    return d(e);
  }
  /**
   * Return debug performance counters for browser-side perf harnesses.
   * @returns {any}
   */
  get_performance_snapshot() {
    const e = c.webviewer_get_performance_snapshot(this.__wbg_ptr);
    return d(e);
  }
  /**
   * @returns {any}
   */
  get_selection_list() {
    const e = c.webviewer_get_selection_list(this.__wbg_ptr);
    return d(e);
  }
  /**
   * @returns {any}
   */
  get_sequence_data() {
    const e = c.webviewer_get_sequence_data(this.__wbg_ptr);
    return d(e);
  }
  /**
   * Load molecular or map data from bytes.
   * @param {Uint8Array} data
   * @param {string} name
   * @param {string} format
   */
  load_data(e, t, _) {
    try {
      const s = c.__wbindgen_add_to_stack_pointer(-16), u = pe(e, c.__wbindgen_export), p = g, W = l(t, c.__wbindgen_export, c.__wbindgen_export2), Y = g, Z = l(_, c.__wbindgen_export, c.__wbindgen_export2), J = g;
      c.webviewer_load_data(s, this.__wbg_ptr, u, p, W, Y, Z, J);
      var r = w().getInt32(s + 0, !0), a = w().getInt32(s + 4, !0);
      if (a)
        throw d(r);
    } finally {
      c.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Returns true when the scene has changed and needs a re-render.
   * @returns {boolean}
   */
  needs_redraw() {
    return c.webviewer_needs_redraw(this.__wbg_ptr) !== 0;
  }
  /**
   * @param {number} x
   * @param {number} y
   * @param {number} button
   * @param {number} modifiers
   */
  on_mouse_down(e, t, _, r) {
    c.webviewer_on_mouse_down(this.__wbg_ptr, e, t, _, r);
  }
  /**
   * @param {number} x
   * @param {number} y
   * @param {number} modifiers
   */
  on_mouse_move(e, t, _) {
    c.webviewer_on_mouse_move(this.__wbg_ptr, e, t, _);
  }
  /**
   * @param {number} x
   * @param {number} y
   * @param {number} button
   */
  on_mouse_up(e, t, _) {
    c.webviewer_on_mouse_up(this.__wbg_ptr, e, t, _);
  }
  /**
   * @param {number} delta_y
   * @param {number} modifiers
   */
  on_wheel(e, t) {
    c.webviewer_on_wheel(this.__wbg_ptr, e, t);
  }
  /**
   * Submit a GPU click pick at physical-pixel canvas coordinates.
   * Returns `null` immediately — the actual hit lands asynchronously
   * via `take_completed_pick()`.
   * @param {number} screen_x
   * @param {number} screen_y
   * @returns {any}
   */
  pick_at_screen(e, t) {
    const _ = c.webviewer_pick_at_screen(this.__wbg_ptr, e, t);
    return d(_);
  }
  /**
   * Try to drain any in-flight GPU picks. JS calls this every rAF so
   * readbacks complete even when no visible redraw is pending.
   */
  poll_pending_picks() {
    c.webviewer_poll_pending_picks(this.__wbg_ptr);
  }
  /**
   * Update hover indicators by submitting a GPU pick at physical-pixel
   * coordinates.
   * @param {number} screen_x
   * @param {number} screen_y
   */
  process_hover(e, t) {
    c.webviewer_process_hover(this.__wbg_ptr, e, t);
  }
  /**
   * Process accumulated input deltas and update the camera.
   */
  process_input() {
    c.webviewer_process_input(this.__wbg_ptr);
  }
  /**
   * Render one frame to the canvas.
   */
  render_frame() {
    c.webviewer_render_frame(this.__wbg_ptr);
  }
  /**
   * Clear debug performance counters for the next harness scenario.
   */
  reset_performance_stats() {
    c.webviewer_reset_performance_stats(this.__wbg_ptr);
  }
  /**
   * Handle canvas resize.
   * @param {number} width
   * @param {number} height
   */
  resize(e, t) {
    c.webviewer_resize(this.__wbg_ptr, e, t);
  }
  /**
   * Enable or disable cursor-based atom picking (default: disabled).
   * @param {boolean} enabled
   */
  set_picking_enabled(e) {
    c.webviewer_set_picking_enabled(this.__wbg_ptr, e);
  }
  /**
   * Enable or disable the visible selection / hover overlay. Hit-test
   * picking remains controlled by `set_picking_enabled`.
   * @param {boolean} enabled
   */
  set_selection_overlay_enabled(e) {
    c.webviewer_set_selection_overlay_enabled(this.__wbg_ptr, e);
  }
  /**
   * Drain renderer warnings produced outside direct command execution.
   * @returns {any}
   */
  takeWarnings() {
    const e = c.webviewer_takeWarnings(this.__wbg_ptr);
    return d(e);
  }
  /**
   * Drain the most recent GPU click pick result.
   * @returns {any}
   */
  take_completed_pick() {
    const e = c.webviewer_take_completed_pick(this.__wbg_ptr);
    return d(e);
  }
  /**
   * Advance movie playback, rock animation, and camera interpolation.
   * @param {number} dt
   */
  update_animations(e) {
    c.webviewer_update_animations(this.__wbg_ptr, e);
  }
}
Symbol.dispose && (T.prototype[Symbol.dispose] = T.prototype.free);
function he() {
  c.init();
}
function H() {
  return {
    __proto__: null,
    "./patinae_web_bg.js": {
      __proto__: null,
      __wbg_Error_83742b46f01ce22d: function(e, t) {
        const _ = Error(f(e, t));
        return i(_);
      },
      __wbg_Window_a07901001eb4269f: function(e) {
        const t = n(e).Window;
        return i(t);
      },
      __wbg_WorkerGlobalScope_d1b9459d53a39f3d: function(e) {
        const t = n(e).WorkerGlobalScope;
        return i(t);
      },
      __wbg___wbindgen_debug_string_5398f5bb970e0daa: function(e, t) {
        const _ = R(n(t)), r = l(_, c.__wbindgen_export, c.__wbindgen_export2), a = g;
        w().setInt32(e + 4, a, !0), w().setInt32(e + 0, r, !0);
      },
      __wbg___wbindgen_is_function_3c846841762788c1: function(e) {
        return typeof n(e) == "function";
      },
      __wbg___wbindgen_is_null_0b605fc6b167c56f: function(e) {
        return n(e) === null;
      },
      __wbg___wbindgen_is_undefined_52709e72fb9f179c: function(e) {
        return n(e) === void 0;
      },
      __wbg___wbindgen_throw_6ddd609b62940d55: function(e, t) {
        throw new Error(f(e, t));
      },
      __wbg__wbg_cb_unref_6b5b6b8576d35cb1: function(e) {
        n(e)._wbg_cb_unref();
      },
      __wbg_beginComputePass_705eb14eefc2b94e: function(e, t) {
        const _ = n(e).beginComputePass(n(t));
        return i(_);
      },
      __wbg_beginRenderPass_10e1d8bb36f2f74e: function() {
        return b(function(e, t) {
          const _ = n(e).beginRenderPass(n(t));
          return i(_);
        }, arguments);
      },
      __wbg_call_2d781c1f4d5c0ef8: function() {
        return b(function(e, t, _) {
          const r = n(e).call(n(t), n(_));
          return i(r);
        }, arguments);
      },
      __wbg_clearBuffer_700f6bba0d974e6c: function(e, t, _) {
        n(e).clearBuffer(n(t), _);
      },
      __wbg_clearBuffer_b67061873f997b6a: function(e, t, _, r) {
        n(e).clearBuffer(n(t), _, r);
      },
      __wbg_configure_3d64c677c7d68a15: function() {
        return b(function(e, t) {
          n(e).configure(n(t));
        }, arguments);
      },
      __wbg_copyBufferToBuffer_8bb974c7f9c5f4dc: function() {
        return b(function(e, t, _, r, a) {
          n(e).copyBufferToBuffer(n(t), _, n(r), a);
        }, arguments);
      },
      __wbg_copyBufferToBuffer_8fe240a0000c9e22: function() {
        return b(function(e, t, _, r, a, s) {
          n(e).copyBufferToBuffer(n(t), _, n(r), a, s);
        }, arguments);
      },
      __wbg_copyTextureToBuffer_4186c16aef1922a5: function() {
        return b(function(e, t, _, r) {
          n(e).copyTextureToBuffer(n(t), n(_), n(r));
        }, arguments);
      },
      __wbg_copyTextureToTexture_1be188df1e535c0a: function() {
        return b(function(e, t, _, r) {
          n(e).copyTextureToTexture(n(t), n(_), n(r));
        }, arguments);
      },
      __wbg_createBindGroupLayout_9ea1a44942aaf13e: function() {
        return b(function(e, t) {
          const _ = n(e).createBindGroupLayout(n(t));
          return i(_);
        }, arguments);
      },
      __wbg_createBindGroup_2320df4db188406c: function(e, t) {
        const _ = n(e).createBindGroup(n(t));
        return i(_);
      },
      __wbg_createBuffer_2f08c0205e04efca: function() {
        return b(function(e, t) {
          const _ = n(e).createBuffer(n(t));
          return i(_);
        }, arguments);
      },
      __wbg_createCommandEncoder_cd88faca35d9ed68: function(e, t) {
        const _ = n(e).createCommandEncoder(n(t));
        return i(_);
      },
      __wbg_createComputePipeline_3e135ff73c8fc483: function(e, t) {
        const _ = n(e).createComputePipeline(n(t));
        return i(_);
      },
      __wbg_createPipelineLayout_7a186f2e9bf0d605: function(e, t) {
        const _ = n(e).createPipelineLayout(n(t));
        return i(_);
      },
      __wbg_createRenderPipeline_f48187ba9f7701e8: function() {
        return b(function(e, t) {
          const _ = n(e).createRenderPipeline(n(t));
          return i(_);
        }, arguments);
      },
      __wbg_createSampler_248bd67c920af37d: function(e, t) {
        const _ = n(e).createSampler(n(t));
        return i(_);
      },
      __wbg_createShaderModule_53701de4fb271c90: function(e, t) {
        const _ = n(e).createShaderModule(n(t));
        return i(_);
      },
      __wbg_createTexture_9e76b80a2dc0d12e: function() {
        return b(function(e, t) {
          const _ = n(e).createTexture(n(t));
          return i(_);
        }, arguments);
      },
      __wbg_createView_cc96b5bdd3d5bf5e: function() {
        return b(function(e, t) {
          const _ = n(e).createView(n(t));
          return i(_);
        }, arguments);
      },
      __wbg_debug_4b9b1a2d5972be57: function(e) {
        console.debug(n(e));
      },
      __wbg_description_18d0a6d4077fec8e: function(e, t) {
        const _ = n(t).description, r = l(_, c.__wbindgen_export, c.__wbindgen_export2), a = g;
        w().setInt32(e + 4, a, !0), w().setInt32(e + 0, r, !0);
      },
      __wbg_dispatchWorkgroups_0cf298d736b85a78: function(e, t, _, r) {
        n(e).dispatchWorkgroups(t >>> 0, _ >>> 0, r >>> 0);
      },
      __wbg_document_c0320cd4183c6d9b: function(e) {
        const t = n(e).document;
        return m(t) ? 0 : i(t);
      },
      __wbg_drawIndexed_68637ebab6dd8d6e: function(e, t, _, r, a, s) {
        n(e).drawIndexed(t >>> 0, _ >>> 0, r >>> 0, a, s >>> 0);
      },
      __wbg_drawIndirect_3cabcd983032eced: function(e, t, _) {
        n(e).drawIndirect(n(t), _);
      },
      __wbg_draw_ad0811de56a2d768: function(e, t, _, r, a) {
        n(e).draw(t >>> 0, _ >>> 0, r >>> 0, a >>> 0);
      },
      __wbg_end_414453a89205612c: function(e) {
        n(e).end();
      },
      __wbg_end_fb560a3ae8e3624e: function(e) {
        n(e).end();
      },
      __wbg_error_8d9a8e04cd1d3588: function(e) {
        console.error(n(e));
      },
      __wbg_error_a6fa202b58aa1cd3: function(e, t) {
        let _, r;
        try {
          _ = e, r = t, console.error(f(e, t));
        } finally {
          c.__wbindgen_export4(_, r, 1);
        }
      },
      __wbg_finish_087cb89c65c06eb1: function(e) {
        const t = n(e).finish();
        return i(t);
      },
      __wbg_finish_cfaeede3baf55be1: function(e, t) {
        const _ = n(e).finish(n(t));
        return i(_);
      },
      __wbg_getContext_a9236f98f1f7fe7c: function() {
        return b(function(e, t, _) {
          const r = n(e).getContext(f(t, _));
          return m(r) ? 0 : i(r);
        }, arguments);
      },
      __wbg_getContext_f04bf8f22dcb2d53: function() {
        return b(function(e, t, _) {
          const r = n(e).getContext(f(t, _));
          return m(r) ? 0 : i(r);
        }, arguments);
      },
      __wbg_getCurrentTexture_51975ae7185fd15f: function() {
        return b(function(e) {
          const t = n(e).getCurrentTexture();
          return i(t);
        }, arguments);
      },
      __wbg_getElementById_d1f25d287b19a833: function(e, t, _) {
        const r = n(e).getElementById(f(t, _));
        return m(r) ? 0 : i(r);
      },
      __wbg_getMappedRange_5ed22727c9679168: function() {
        return b(function(e, t, _) {
          const r = n(e).getMappedRange(t, _);
          return i(r);
        }, arguments);
      },
      __wbg_getPreferredCanvasFormat_1b8495aeb1d11ab1: function(e) {
        const t = n(e).getPreferredCanvasFormat();
        return (y.indexOf(t) + 1 || 96) - 1;
      },
      __wbg_getRandomValues_3f44b700395062e5: function() {
        return b(function(e, t) {
          globalThis.crypto.getRandomValues(k(e, t));
        }, arguments);
      },
      __wbg_get_c7546417fb0bec10: function(e, t) {
        const _ = n(e)[t >>> 0];
        return m(_) ? 0 : i(_);
      },
      __wbg_gpu_a7c12045c25d009a: function(e) {
        const t = n(e).gpu;
        return i(t);
      },
      __wbg_height_6568c4427c3b889d: function(e) {
        return n(e).height;
      },
      __wbg_info_22dcf1fd1b12bc7d: function(e) {
        const t = n(e).info;
        return i(t);
      },
      __wbg_info_7d4e223bb1a7e671: function(e) {
        console.info(n(e));
      },
      __wbg_instanceof_GpuAdapter_fc7b89fc546de0bc: function(e) {
        let t;
        try {
          t = n(e) instanceof GPUAdapter;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_instanceof_GpuCanvasContext_1a39fd0621603553: function(e) {
        let t;
        try {
          t = n(e) instanceof GPUCanvasContext;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_instanceof_HtmlCanvasElement_26125339f936be50: function(e) {
        let t;
        try {
          t = n(e) instanceof HTMLCanvasElement;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_instanceof_Window_23e677d2c6843922: function(e) {
        let t;
        try {
          t = n(e) instanceof Window;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_label_47480289cc2bce71: function(e, t) {
        const _ = n(t).label, r = l(_, c.__wbindgen_export, c.__wbindgen_export2), a = g;
        w().setInt32(e + 4, a, !0), w().setInt32(e + 0, r, !0);
      },
      __wbg_length_ea16607d7b61445b: function(e) {
        return n(e).length;
      },
      __wbg_limits_1c25cb4f379a4418: function(e) {
        const t = n(e).limits;
        return i(t);
      },
      __wbg_limits_50a8c5e629dbfe40: function(e) {
        const t = n(e).limits;
        return i(t);
      },
      __wbg_log_524eedafa26daa59: function(e) {
        console.log(n(e));
      },
      __wbg_mapAsync_bb0029907dd91181: function(e, t, _, r) {
        const a = n(e).mapAsync(t >>> 0, _, r);
        return i(a);
      },
      __wbg_maxBindGroups_14611ac9ed1c6b56: function(e) {
        return n(e).maxBindGroups;
      },
      __wbg_maxBindingsPerBindGroup_dd3f66044d2a9bfb: function(e) {
        return n(e).maxBindingsPerBindGroup;
      },
      __wbg_maxBufferSize_f7ce3e1856349d2f: function(e) {
        return n(e).maxBufferSize;
      },
      __wbg_maxColorAttachmentBytesPerSample_55e64194645ea041: function(e) {
        return n(e).maxColorAttachmentBytesPerSample;
      },
      __wbg_maxColorAttachments_fd9187f9f786da18: function(e) {
        return n(e).maxColorAttachments;
      },
      __wbg_maxComputeInvocationsPerWorkgroup_9b3b1fc261129782: function(e) {
        return n(e).maxComputeInvocationsPerWorkgroup;
      },
      __wbg_maxComputeWorkgroupSizeX_c55bbbcc02b75241: function(e) {
        return n(e).maxComputeWorkgroupSizeX;
      },
      __wbg_maxComputeWorkgroupSizeY_96f40b1ec3102a3a: function(e) {
        return n(e).maxComputeWorkgroupSizeY;
      },
      __wbg_maxComputeWorkgroupSizeZ_c2b1061d521561bb: function(e) {
        return n(e).maxComputeWorkgroupSizeZ;
      },
      __wbg_maxComputeWorkgroupStorageSize_fac26e89d99e08f9: function(e) {
        return n(e).maxComputeWorkgroupStorageSize;
      },
      __wbg_maxComputeWorkgroupsPerDimension_cd001f910e9b4d70: function(e) {
        return n(e).maxComputeWorkgroupsPerDimension;
      },
      __wbg_maxDynamicStorageBuffersPerPipelineLayout_29399b82af020d86: function(e) {
        return n(e).maxDynamicStorageBuffersPerPipelineLayout;
      },
      __wbg_maxDynamicUniformBuffersPerPipelineLayout_6d6cf80f3bd08e52: function(e) {
        return n(e).maxDynamicUniformBuffersPerPipelineLayout;
      },
      __wbg_maxInterStageShaderVariables_8b000f47a166b1d5: function(e) {
        return n(e).maxInterStageShaderVariables;
      },
      __wbg_maxSampledTexturesPerShaderStage_618a49f33217dde2: function(e) {
        return n(e).maxSampledTexturesPerShaderStage;
      },
      __wbg_maxSamplersPerShaderStage_aa09fa0311712a1a: function(e) {
        return n(e).maxSamplersPerShaderStage;
      },
      __wbg_maxStorageBufferBindingSize_0ec83ae10ad73180: function(e) {
        return n(e).maxStorageBufferBindingSize;
      },
      __wbg_maxStorageBuffersPerShaderStage_0cca5b468fcf10b6: function(e) {
        return n(e).maxStorageBuffersPerShaderStage;
      },
      __wbg_maxStorageTexturesPerShaderStage_9d6c35770f37866c: function(e) {
        return n(e).maxStorageTexturesPerShaderStage;
      },
      __wbg_maxTextureArrayLayers_c2bf9c85285832d4: function(e) {
        return n(e).maxTextureArrayLayers;
      },
      __wbg_maxTextureDimension1D_e09f86e22ea6bac9: function(e) {
        return n(e).maxTextureDimension1D;
      },
      __wbg_maxTextureDimension2D_2631916ef9a3efa8: function(e) {
        return n(e).maxTextureDimension2D;
      },
      __wbg_maxTextureDimension3D_06ee54121b37d431: function(e) {
        return n(e).maxTextureDimension3D;
      },
      __wbg_maxUniformBufferBindingSize_af9e8a077907ed64: function(e) {
        return n(e).maxUniformBufferBindingSize;
      },
      __wbg_maxUniformBuffersPerShaderStage_f871b70865df8c11: function(e) {
        return n(e).maxUniformBuffersPerShaderStage;
      },
      __wbg_maxVertexAttributes_e72dabb2714f5cf5: function(e) {
        return n(e).maxVertexAttributes;
      },
      __wbg_maxVertexBufferArrayStride_6a1cd814386082ce: function(e) {
        return n(e).maxVertexBufferArrayStride;
      },
      __wbg_maxVertexBuffers_9c61c5fd286ebcc6: function(e) {
        return n(e).maxVertexBuffers;
      },
      __wbg_minStorageBufferOffsetAlignment_e214f59628fb3558: function(e) {
        return n(e).minStorageBufferOffsetAlignment;
      },
      __wbg_minUniformBufferOffsetAlignment_58b69e1c3924f6a4: function(e) {
        return n(e).minUniformBufferOffsetAlignment;
      },
      __wbg_navigator_583ffd4fc14c0f7a: function(e) {
        const t = n(e).navigator;
        return i(t);
      },
      __wbg_navigator_9cebf56f28aa719b: function(e) {
        const t = n(e).navigator;
        return i(t);
      },
      __wbg_new_227d7c05414eb861: function() {
        const e = new Error();
        return i(e);
      },
      __wbg_new_a70fbab9066b301f: function() {
        const e = new Array();
        return i(e);
      },
      __wbg_new_ab79df5bd7c26067: function() {
        const e = new Object();
        return i(e);
      },
      __wbg_new_typed_aaaeaf29cf802876: function(e, t) {
        try {
          var _ = { a: e, b: t }, r = (s, u) => {
            const p = _.a;
            _.a = 0;
            try {
              return ee(p, _.b, s, u);
            } finally {
              _.a = p;
            }
          };
          const a = new Promise(r);
          return i(a);
        } finally {
          _.a = _.b = 0;
        }
      },
      __wbg_new_typed_bccac67128ed885a: function() {
        const e = new Array();
        return i(e);
      },
      __wbg_new_with_byte_offset_and_length_b2ec5bf7b2f35743: function(e, t, _) {
        const r = new Uint8Array(n(e), t >>> 0, _ >>> 0);
        return i(r);
      },
      __wbg_now_c6d7a7d35f74f6f1: function(e) {
        return n(e).now();
      },
      __wbg_onSubmittedWorkDone_1460145eecea40ef: function(e) {
        const t = n(e).onSubmittedWorkDone();
        return i(t);
      },
      __wbg_performance_28be169151161678: function(e) {
        const t = n(e).performance;
        return m(t) ? 0 : i(t);
      },
      __wbg_prototypesetcall_d62e5099504357e6: function(e, t, _) {
        Uint8Array.prototype.set.call(k(e, t), n(_));
      },
      __wbg_push_e87b0e732085a946: function(e, t) {
        return n(e).push(n(t));
      },
      __wbg_querySelectorAll_ccbf0696a1c6fed8: function() {
        return b(function(e, t, _) {
          const r = n(e).querySelectorAll(f(t, _));
          return i(r);
        }, arguments);
      },
      __wbg_queueMicrotask_0c399741342fb10f: function(e) {
        const t = n(e).queueMicrotask;
        return i(t);
      },
      __wbg_queueMicrotask_a082d78ce798393e: function(e) {
        queueMicrotask(n(e));
      },
      __wbg_queue_65d985f3e6d786a6: function(e) {
        const t = n(e).queue;
        return i(t);
      },
      __wbg_requestAdapter_9ff5c9d1ff271165: function(e, t) {
        const _ = n(e).requestAdapter(n(t));
        return i(_);
      },
      __wbg_requestDevice_c1c34f88a477e509: function(e, t) {
        const _ = n(e).requestDevice(n(t));
        return i(_);
      },
      __wbg_resolve_ae8d83246e5bcc12: function(e) {
        const t = Promise.resolve(n(e));
        return i(t);
      },
      __wbg_setBindGroup_4ba56e1e0d26f244: function() {
        return b(function(e, t, _, r, a, s, u) {
          n(e).setBindGroup(t >>> 0, n(_), $(r, a), s, u >>> 0);
        }, arguments);
      },
      __wbg_setBindGroup_6124849cc8547086: function(e, t, _) {
        n(e).setBindGroup(t >>> 0, n(_));
      },
      __wbg_setBindGroup_79afcff8b9db8be3: function() {
        return b(function(e, t, _, r, a, s, u) {
          n(e).setBindGroup(t >>> 0, n(_), $(r, a), s, u >>> 0);
        }, arguments);
      },
      __wbg_setBindGroup_84eb639ac393a9f4: function(e, t, _) {
        n(e).setBindGroup(t >>> 0, n(_));
      },
      __wbg_setIndexBuffer_17431786d06c1b7c: function(e, t, _, r, a) {
        n(e).setIndexBuffer(n(t), I[_], r, a);
      },
      __wbg_setIndexBuffer_a16ed5b869c87507: function(e, t, _, r) {
        n(e).setIndexBuffer(n(t), I[_], r);
      },
      __wbg_setPipeline_95c76ab8da697fcf: function(e, t) {
        n(e).setPipeline(n(t));
      },
      __wbg_setPipeline_bab24dbce96903b9: function(e, t) {
        n(e).setPipeline(n(t));
      },
      __wbg_setScissorRect_40786fdec122b032: function(e, t, _, r, a) {
        n(e).setScissorRect(t >>> 0, _ >>> 0, r >>> 0, a >>> 0);
      },
      __wbg_setVertexBuffer_91c4b602d0289943: function(e, t, _, r) {
        n(e).setVertexBuffer(t >>> 0, n(_), r);
      },
      __wbg_setVertexBuffer_b508baf8d0ffe331: function(e, t, _, r, a) {
        n(e).setVertexBuffer(t >>> 0, n(_), r, a);
      },
      __wbg_setViewport_f9d423db4f4b4b58: function(e, t, _, r, a, s, u) {
        n(e).setViewport(t, _, r, a, s, u);
      },
      __wbg_set_282384002438957f: function(e, t, _) {
        n(e)[t >>> 0] = d(_);
      },
      __wbg_set_6be42768c690e380: function(e, t, _) {
        n(e)[d(t)] = d(_);
      },
      __wbg_set_7eaa4f96924fd6b3: function() {
        return b(function(e, t, _) {
          return Reflect.set(n(e), n(t), n(_));
        }, arguments);
      },
      __wbg_set_a_5f6e488475272136: function(e, t) {
        n(e).a = t;
      },
      __wbg_set_access_091f317905cd76a5: function(e, t) {
        n(e).access = ue[t];
      },
      __wbg_set_address_mode_u_a37cf1035585c638: function(e, t) {
        n(e).addressModeU = O[t];
      },
      __wbg_set_address_mode_v_8ac049e029caef76: function(e, t) {
        n(e).addressModeV = O[t];
      },
      __wbg_set_address_mode_w_eb9260ee11729e92: function(e, t) {
        n(e).addressModeW = O[t];
      },
      __wbg_set_alpha_aa2e606e9e647b21: function(e, t) {
        n(e).alpha = n(t);
      },
      __wbg_set_alpha_mode_92402195b3ae1ee7: function(e, t) {
        n(e).alphaMode = _e[t];
      },
      __wbg_set_alpha_to_coverage_enabled_b4ce9c3f7f8b7ad7: function(e, t) {
        n(e).alphaToCoverageEnabled = t !== 0;
      },
      __wbg_set_array_layer_count_daec613068108a9d: function(e, t) {
        n(e).arrayLayerCount = t >>> 0;
      },
      __wbg_set_array_stride_c2c009eabc18b5f6: function(e, t) {
        n(e).arrayStride = t;
      },
      __wbg_set_aspect_77332ac136ee94eb: function(e, t) {
        n(e).aspect = j[t];
      },
      __wbg_set_aspect_a823a14d00d42d37: function(e, t) {
        n(e).aspect = j[t];
      },
      __wbg_set_attributes_05f9117fd32ca606: function(e, t) {
        n(e).attributes = n(t);
      },
      __wbg_set_b_688365d692bba214: function(e, t) {
        n(e).b = t;
      },
      __wbg_set_base_array_layer_cc6c68d233489c4b: function(e, t) {
        n(e).baseArrayLayer = t >>> 0;
      },
      __wbg_set_base_mip_level_e07a3efe9006d5ea: function(e, t) {
        n(e).baseMipLevel = t >>> 0;
      },
      __wbg_set_beginning_of_pass_write_index_27be5b0b35ec3de0: function(e, t) {
        n(e).beginningOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_beginning_of_pass_write_index_c12e7856ee670800: function(e, t) {
        n(e).beginningOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_bind_group_layouts_5325d038771af328: function(e, t) {
        n(e).bindGroupLayouts = n(t);
      },
      __wbg_set_binding_b6b0fe5c281b8c69: function(e, t) {
        n(e).binding = t >>> 0;
      },
      __wbg_set_binding_f3c188a8cd21455b: function(e, t) {
        n(e).binding = t >>> 0;
      },
      __wbg_set_blend_8d6e9c08b5702a09: function(e, t) {
        n(e).blend = n(t);
      },
      __wbg_set_buffer_55f096330c8912b4: function(e, t) {
        n(e).buffer = n(t);
      },
      __wbg_set_buffer_aa7bf4ad8f17b2bd: function(e, t) {
        n(e).buffer = n(t);
      },
      __wbg_set_buffer_e89095a9f0cafad3: function(e, t) {
        n(e).buffer = n(t);
      },
      __wbg_set_buffers_85a7238f4ef28ab4: function(e, t) {
        n(e).buffers = n(t);
      },
      __wbg_set_bytes_per_row_68a1ea90d4710bc9: function(e, t) {
        n(e).bytesPerRow = t >>> 0;
      },
      __wbg_set_clear_value_642701f928a5ccb3: function(e, t) {
        n(e).clearValue = n(t);
      },
      __wbg_set_code_56e2d45ec1ff6c2d: function(e, t, _) {
        n(e).code = f(t, _);
      },
      __wbg_set_color_attachments_abe67f6631926e28: function(e, t) {
        n(e).colorAttachments = n(t);
      },
      __wbg_set_color_bc393d7efc3c8594: function(e, t) {
        n(e).color = n(t);
      },
      __wbg_set_compare_1509dc1a5420943f: function(e, t) {
        n(e).compare = G[t];
      },
      __wbg_set_compare_42211fbf15e3b850: function(e, t) {
        n(e).compare = G[t];
      },
      __wbg_set_compute_5a859e405c9eb6c6: function(e, t) {
        n(e).compute = n(t);
      },
      __wbg_set_count_26a934d1cd07d080: function(e, t) {
        n(e).count = t >>> 0;
      },
      __wbg_set_cull_mode_9d466c1ab414cac8: function(e, t) {
        n(e).cullMode = re[t];
      },
      __wbg_set_depth_bias_428c9340b0fd937b: function(e, t) {
        n(e).depthBias = t;
      },
      __wbg_set_depth_bias_clamp_f009599ca67fa30c: function(e, t) {
        n(e).depthBiasClamp = t;
      },
      __wbg_set_depth_bias_slope_scale_7125880b4cb7a951: function(e, t) {
        n(e).depthBiasSlopeScale = t;
      },
      __wbg_set_depth_clear_value_442bf492734f63b6: function(e, t) {
        n(e).depthClearValue = t;
      },
      __wbg_set_depth_compare_30e9ea552da12fe2: function(e, t) {
        n(e).depthCompare = G[t];
      },
      __wbg_set_depth_fail_op_5e42dc3e4c382951: function(e, t) {
        n(e).depthFailOp = D[t];
      },
      __wbg_set_depth_load_op_34d430b74bb36d91: function(e, t) {
        n(e).depthLoadOp = M[t];
      },
      __wbg_set_depth_or_array_layers_4bbbeadacb393f02: function(e, t) {
        n(e).depthOrArrayLayers = t >>> 0;
      },
      __wbg_set_depth_read_only_138a11b10c731094: function(e, t) {
        n(e).depthReadOnly = t !== 0;
      },
      __wbg_set_depth_stencil_1bd50dbc450c8650: function(e, t) {
        n(e).depthStencil = n(t);
      },
      __wbg_set_depth_stencil_attachment_1ee0d93bc3273369: function(e, t) {
        n(e).depthStencilAttachment = n(t);
      },
      __wbg_set_depth_store_op_0ea0a215313dbda7: function(e, t) {
        n(e).depthStoreOp = F[t];
      },
      __wbg_set_depth_write_enabled_64c2e7f6fa4b6b7b: function(e, t) {
        n(e).depthWriteEnabled = t !== 0;
      },
      __wbg_set_device_0d774b66e7288f72: function(e, t) {
        n(e).device = n(t);
      },
      __wbg_set_dimension_174ad7e2fb67fb4e: function(e, t) {
        n(e).dimension = L[t];
      },
      __wbg_set_dimension_36e13ccecae5af4b: function(e, t) {
        n(e).dimension = fe[t];
      },
      __wbg_set_dst_factor_1ed75271a89a711e: function(e, t) {
        n(e).dstFactor = V[t];
      },
      __wbg_set_e80615d7a9a43981: function(e, t, _) {
        n(e).set(n(t), _ >>> 0);
      },
      __wbg_set_end_of_pass_write_index_e8f52fc08bc0603e: function(e, t) {
        n(e).endOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_end_of_pass_write_index_f4ab90c5743df805: function(e, t) {
        n(e).endOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_entries_3017e6132f938c6e: function(e, t) {
        n(e).entries = n(t);
      },
      __wbg_set_entries_fc76ca4d7da6a709: function(e, t) {
        n(e).entries = n(t);
      },
      __wbg_set_entry_point_4443daff87d82ef1: function(e, t, _) {
        n(e).entryPoint = f(t, _);
      },
      __wbg_set_entry_point_6fec5723cc790927: function(e, t, _) {
        n(e).entryPoint = f(t, _);
      },
      __wbg_set_entry_point_8db3b6d103e3b865: function(e, t, _) {
        n(e).entryPoint = f(t, _);
      },
      __wbg_set_external_texture_825fe2bc7a0c0603: function(e, t) {
        n(e).externalTexture = n(t);
      },
      __wbg_set_fail_op_77ab26c98f847b65: function(e, t) {
        n(e).failOp = D[t];
      },
      __wbg_set_format_1786adb7bc74c7c9: function(e, t) {
        n(e).format = y[t];
      },
      __wbg_set_format_6606f5c1fba6f459: function(e, t) {
        n(e).format = de[t];
      },
      __wbg_set_format_90860b0321868db4: function(e, t) {
        n(e).format = y[t];
      },
      __wbg_set_format_abf7a1bc5425c56a: function(e, t) {
        n(e).format = y[t];
      },
      __wbg_set_format_d347899cd860709c: function(e, t) {
        n(e).format = y[t];
      },
      __wbg_set_format_e9d4b1475bb3bd3b: function(e, t) {
        n(e).format = y[t];
      },
      __wbg_set_format_f9341112e43ea182: function(e, t) {
        n(e).format = y[t];
      },
      __wbg_set_fragment_1a595620425637e1: function(e, t) {
        n(e).fragment = n(t);
      },
      __wbg_set_front_face_50cdf4eb61504a46: function(e, t) {
        n(e).frontFace = oe[t];
      },
      __wbg_set_g_d4d1d77cf8fdd362: function(e, t) {
        n(e).g = t;
      },
      __wbg_set_has_dynamic_offset_7d30014fdbfe90c5: function(e, t) {
        n(e).hasDynamicOffset = t !== 0;
      },
      __wbg_set_height_98a1a397672657e2: function(e, t) {
        n(e).height = t >>> 0;
      },
      __wbg_set_height_b6548a01bdcb689a: function(e, t) {
        n(e).height = t >>> 0;
      },
      __wbg_set_height_e8b5483b8c117d5e: function(e, t) {
        n(e).height = t >>> 0;
      },
      __wbg_set_label_03d2396d4655a3e1: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_0c1bd0e976cf0a9a: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_1175a3329a06e52b: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_2d2227f4d5991e50: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_2f592bd1be3db6b3: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_4a1dd4244f80abc9: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_8b0da33fd11b2572: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_8fd860a36d2c7b74: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_bae57fb9f24fde5c: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_be45aed56e4b9fee: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_c47c451211e2f6d2: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_cd567b7b35838e4c: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_d1c24b5a7a3ac31d: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_dcd98efbb9370da8: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_label_f92ae11c77d74198: function(e, t, _) {
        n(e).label = f(t, _);
      },
      __wbg_set_layout_19e558a0fa724e95: function(e, t) {
        n(e).layout = n(t);
      },
      __wbg_set_layout_7c5ba5bdcde8a0f0: function(e, t) {
        n(e).layout = n(t);
      },
      __wbg_set_layout_eeef59714f5bf48b: function(e, t) {
        n(e).layout = n(t);
      },
      __wbg_set_load_op_56844f51434037bf: function(e, t) {
        n(e).loadOp = M[t];
      },
      __wbg_set_lod_max_clamp_3f157633f32c9f94: function(e, t) {
        n(e).lodMaxClamp = t;
      },
      __wbg_set_lod_min_clamp_7e246c739fb1a854: function(e, t) {
        n(e).lodMinClamp = t;
      },
      __wbg_set_mag_filter_69d846b974d4bcc0: function(e, t) {
        n(e).magFilter = q[t];
      },
      __wbg_set_mapped_at_creation_48de4735fab51e78: function(e, t) {
        n(e).mappedAtCreation = t !== 0;
      },
      __wbg_set_mask_0c49a66362fc0079: function(e, t) {
        n(e).mask = t >>> 0;
      },
      __wbg_set_max_anisotropy_3ef0d5bca2336cc7: function(e, t) {
        n(e).maxAnisotropy = t;
      },
      __wbg_set_min_binding_size_689661b9ed25e083: function(e, t) {
        n(e).minBindingSize = t;
      },
      __wbg_set_min_filter_fbf2d8d9f503dcd7: function(e, t) {
        n(e).minFilter = q[t];
      },
      __wbg_set_mip_level_246db61be15bdd69: function(e, t) {
        n(e).mipLevel = t >>> 0;
      },
      __wbg_set_mip_level_count_72f8bc1f80f7539b: function(e, t) {
        n(e).mipLevelCount = t >>> 0;
      },
      __wbg_set_mip_level_count_b19a0d9192e62d5d: function(e, t) {
        n(e).mipLevelCount = t >>> 0;
      },
      __wbg_set_mipmap_filter_17fd50a3898fd5ff: function(e, t) {
        n(e).mipmapFilter = ce[t];
      },
      __wbg_set_module_08ad08e736d8edbf: function(e, t) {
        n(e).module = n(t);
      },
      __wbg_set_module_14e471fdd94c582d: function(e, t) {
        n(e).module = n(t);
      },
      __wbg_set_module_9b938909233aed50: function(e, t) {
        n(e).module = n(t);
      },
      __wbg_set_multisample_85f073947b782d07: function(e, t) {
        n(e).multisample = n(t);
      },
      __wbg_set_multisampled_40505c1381e1c32c: function(e, t) {
        n(e).multisampled = t !== 0;
      },
      __wbg_set_offset_2c374e604504e0b2: function(e, t) {
        n(e).offset = t;
      },
      __wbg_set_offset_73156b0e0b41d79a: function(e, t) {
        n(e).offset = t;
      },
      __wbg_set_offset_8d9d9afffa18b591: function(e, t) {
        n(e).offset = t;
      },
      __wbg_set_operation_b5862f5a1a143b30: function(e, t) {
        n(e).operation = te[t];
      },
      __wbg_set_origin_9b3b0fbe0a5dc469: function(e, t) {
        n(e).origin = n(t);
      },
      __wbg_set_pass_op_e9470d1262fb8a8b: function(e, t) {
        n(e).passOp = D[t];
      },
      __wbg_set_power_preference_c0d3fa7ce46b1a2e: function(e, t) {
        n(e).powerPreference = ie[t];
      },
      __wbg_set_primitive_369241acd17871f1: function(e, t) {
        n(e).primitive = n(t);
      },
      __wbg_set_query_set_18679a8580267d5a: function(e, t) {
        n(e).querySet = n(t);
      },
      __wbg_set_query_set_f1314b06c84c4b00: function(e, t) {
        n(e).querySet = n(t);
      },
      __wbg_set_r_527e5a41c4b1a846: function(e, t) {
        n(e).r = t;
      },
      __wbg_set_required_features_54918de8185c5fab: function(e, t) {
        n(e).requiredFeatures = n(t);
      },
      __wbg_set_required_limits_3b031f66f838f4e3: function(e, t) {
        n(e).requiredLimits = n(t);
      },
      __wbg_set_resolve_target_fe76b3f99cf72078: function(e, t) {
        n(e).resolveTarget = n(t);
      },
      __wbg_set_resource_fe385d2e3dadaf63: function(e, t) {
        n(e).resource = n(t);
      },
      __wbg_set_rows_per_image_f9878f4b10f4fd7f: function(e, t) {
        n(e).rowsPerImage = t >>> 0;
      },
      __wbg_set_sample_count_865e1d19b84e27e6: function(e, t) {
        n(e).sampleCount = t >>> 0;
      },
      __wbg_set_sample_type_7088b1efddce6a69: function(e, t) {
        n(e).sampleType = be[t];
      },
      __wbg_set_sampler_8c5d7fb1b02058c6: function(e, t) {
        n(e).sampler = n(t);
      },
      __wbg_set_shader_location_0ff30a733291a396: function(e, t) {
        n(e).shaderLocation = t >>> 0;
      },
      __wbg_set_size_1e6281b07cd39177: function(e, t) {
        n(e).size = t;
      },
      __wbg_set_size_41cd9255ca1e4242: function(e, t) {
        n(e).size = t;
      },
      __wbg_set_size_a61ff22205255d61: function(e, t) {
        n(e).size = n(t);
      },
      __wbg_set_src_factor_1c4f755f8676df1b: function(e, t) {
        n(e).srcFactor = V[t];
      },
      __wbg_set_stencil_back_6ef4683123b19b25: function(e, t) {
        n(e).stencilBack = n(t);
      },
      __wbg_set_stencil_clear_value_10b58f674d0177c2: function(e, t) {
        n(e).stencilClearValue = t >>> 0;
      },
      __wbg_set_stencil_front_aeb8580a97e5424b: function(e, t) {
        n(e).stencilFront = n(t);
      },
      __wbg_set_stencil_load_op_f20a90a66acd3d8c: function(e, t) {
        n(e).stencilLoadOp = M[t];
      },
      __wbg_set_stencil_read_mask_2954f260d47349ea: function(e, t) {
        n(e).stencilReadMask = t >>> 0;
      },
      __wbg_set_stencil_read_only_fb489d191b6d969b: function(e, t) {
        n(e).stencilReadOnly = t !== 0;
      },
      __wbg_set_stencil_store_op_477c4cf6422dfa3f: function(e, t) {
        n(e).stencilStoreOp = F[t];
      },
      __wbg_set_stencil_write_mask_3f8e9b3781814a95: function(e, t) {
        n(e).stencilWriteMask = t >>> 0;
      },
      __wbg_set_step_mode_a35aef328761c452: function(e, t) {
        n(e).stepMode = ge[t];
      },
      __wbg_set_storage_texture_ab9eed9786337ef0: function(e, t) {
        n(e).storageTexture = n(t);
      },
      __wbg_set_store_op_caeede4654b3d847: function(e, t) {
        n(e).storeOp = F[t];
      },
      __wbg_set_strip_index_format_0cd0510e166c4ec4: function(e, t) {
        n(e).stripIndexFormat = I[t];
      },
      __wbg_set_targets_6b0b3bdd87f35668: function(e, t) {
        n(e).targets = n(t);
      },
      __wbg_set_texture_16d2be474ce6ad0c: function(e, t) {
        n(e).texture = n(t);
      },
      __wbg_set_texture_e25a73da75cf5808: function(e, t) {
        n(e).texture = n(t);
      },
      __wbg_set_timestamp_writes_26336a2ad72cdcaf: function(e, t) {
        n(e).timestampWrites = n(t);
      },
      __wbg_set_timestamp_writes_c552d52fbb417005: function(e, t) {
        n(e).timestampWrites = n(t);
      },
      __wbg_set_topology_beefb3aca0612b00: function(e, t) {
        n(e).topology = ae[t];
      },
      __wbg_set_type_38961e08504ca674: function(e, t) {
        n(e).type = ne[t];
      },
      __wbg_set_type_c1eebc19f8a6aeb9: function(e, t) {
        n(e).type = se[t];
      },
      __wbg_set_unclipped_depth_5a4f7eb57fe006b2: function(e, t) {
        n(e).unclippedDepth = t !== 0;
      },
      __wbg_set_usage_7f0dda8309469b1c: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_usage_7fa9cd18d1104aca: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_usage_908213a4d4bb8bde: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_usage_ae014e77ff77ce06: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_vertex_a4951dd9a7a4ed54: function(e, t) {
        n(e).vertex = n(t);
      },
      __wbg_set_view_bdeab150b5f0768c: function(e, t) {
        n(e).view = n(t);
      },
      __wbg_set_view_dbd0294573f64d05: function(e, t) {
        n(e).view = n(t);
      },
      __wbg_set_view_dimension_263387976511ebc9: function(e, t) {
        n(e).viewDimension = L[t];
      },
      __wbg_set_view_dimension_3ed01b237e85826f: function(e, t) {
        n(e).viewDimension = L[t];
      },
      __wbg_set_view_formats_bab284fc81b40e70: function(e, t) {
        n(e).viewFormats = n(t);
      },
      __wbg_set_view_formats_fe531a043efb71fa: function(e, t) {
        n(e).viewFormats = n(t);
      },
      __wbg_set_visibility_1bca121a89accba5: function(e, t) {
        n(e).visibility = t >>> 0;
      },
      __wbg_set_width_1a5e2e86fa5bdcd8: function(e, t) {
        n(e).width = t >>> 0;
      },
      __wbg_set_width_576343a4a7f2cf28: function(e, t) {
        n(e).width = t >>> 0;
      },
      __wbg_set_width_c0fcaa2da53cd540: function(e, t) {
        n(e).width = t >>> 0;
      },
      __wbg_set_write_mask_144b25e2bd909124: function(e, t) {
        n(e).writeMask = t >>> 0;
      },
      __wbg_set_x_56f0c2c08a62725c: function(e, t) {
        n(e).x = t >>> 0;
      },
      __wbg_set_y_04fb8ce84735b4e1: function(e, t) {
        n(e).y = t >>> 0;
      },
      __wbg_set_z_a51316db27a4941e: function(e, t) {
        n(e).z = t >>> 0;
      },
      __wbg_stack_3b0d974bbf31e44f: function(e, t) {
        const _ = n(t).stack, r = l(_, c.__wbindgen_export, c.__wbindgen_export2), a = g;
        w().setInt32(e + 4, a, !0), w().setInt32(e + 0, r, !0);
      },
      __wbg_static_accessor_GLOBAL_8adb955bd33fac2f: function() {
        const e = typeof global > "u" ? null : global;
        return m(e) ? 0 : i(e);
      },
      __wbg_static_accessor_GLOBAL_THIS_ad356e0db91c7913: function() {
        const e = typeof globalThis > "u" ? null : globalThis;
        return m(e) ? 0 : i(e);
      },
      __wbg_static_accessor_SELF_f207c857566db248: function() {
        const e = typeof self > "u" ? null : self;
        return m(e) ? 0 : i(e);
      },
      __wbg_static_accessor_WINDOW_bb9f1ba69d61b386: function() {
        const e = typeof window > "u" ? null : window;
        return m(e) ? 0 : i(e);
      },
      __wbg_submit_1290d44bb76ecef4: function(e, t) {
        n(e).submit(n(t));
      },
      __wbg_then_098abe61755d12f6: function(e, t) {
        const _ = n(e).then(n(t));
        return i(_);
      },
      __wbg_then_9e335f6dd892bc11: function(e, t, _) {
        const r = n(e).then(n(t), n(_));
        return i(r);
      },
      __wbg_then_bc59d1943397ca4e: function(e, t, _) {
        const r = n(e).then(n(t), n(_));
        return i(r);
      },
      __wbg_unmap_8f06698a75b8331a: function(e) {
        n(e).unmap();
      },
      __wbg_warn_69424c2d92a2fa73: function(e) {
        console.warn(n(e));
      },
      __wbg_webviewer_new: function(e) {
        const t = T.__wrap(e);
        return i(t);
      },
      __wbg_width_4d6fc7fecd877217: function(e) {
        return n(e).width;
      },
      __wbg_writeBuffer_b4bdd36178348ca5: function() {
        return b(function(e, t, _, r, a, s, u) {
          n(e).writeBuffer(n(t), _, k(r, a), s, u);
        }, arguments);
      },
      __wbindgen_cast_0000000000000001: function(e, t) {
        const _ = N(e, t, c.__wasm_bindgen_func_elem_13856, K);
        return i(_);
      },
      __wbindgen_cast_0000000000000002: function(e, t) {
        const _ = N(e, t, c.__wasm_bindgen_func_elem_14781, Q);
        return i(_);
      },
      __wbindgen_cast_0000000000000003: function(e) {
        return i(e);
      },
      __wbindgen_cast_0000000000000004: function(e, t) {
        const _ = k(e, t);
        return i(_);
      },
      __wbindgen_cast_0000000000000005: function(e, t) {
        const _ = f(e, t);
        return i(_);
      },
      __wbindgen_cast_0000000000000006: function(e) {
        const t = BigInt.asUintN(64, e);
        return i(t);
      },
      __wbindgen_object_clone_ref: function(e) {
        const t = n(e);
        return i(t);
      },
      __wbindgen_object_drop_ref: function(e) {
        d(e);
      }
    }
  };
}
function K(o, e, t) {
  c.__wasm_bindgen_func_elem_13858(o, e, i(t));
}
function Q(o, e, t) {
  try {
    const a = c.__wbindgen_add_to_stack_pointer(-16);
    c.__wasm_bindgen_func_elem_15346(a, o, e, i(t));
    var _ = w().getInt32(a + 0, !0), r = w().getInt32(a + 4, !0);
    if (r)
      throw d(_);
  } finally {
    c.__wbindgen_add_to_stack_pointer(16);
  }
}
function ee(o, e, t, _) {
  c.__wasm_bindgen_func_elem_15348(o, e, i(t), i(_));
}
const O = ["clamp-to-edge", "repeat", "mirror-repeat"], V = ["zero", "one", "src", "one-minus-src", "src-alpha", "one-minus-src-alpha", "dst", "one-minus-dst", "dst-alpha", "one-minus-dst-alpha", "src-alpha-saturated", "constant", "one-minus-constant", "src1", "one-minus-src1", "src1-alpha", "one-minus-src1-alpha"], te = ["add", "subtract", "reverse-subtract", "min", "max"], ne = ["uniform", "storage", "read-only-storage"], _e = ["opaque", "premultiplied"], G = ["never", "less", "equal", "less-equal", "greater", "not-equal", "greater-equal", "always"], re = ["none", "front", "back"], q = ["nearest", "linear"], oe = ["ccw", "cw"], I = ["uint16", "uint32"], M = ["load", "clear"], ce = ["nearest", "linear"], ie = ["low-power", "high-performance"], ae = ["point-list", "line-list", "line-strip", "triangle-list", "triangle-strip"], se = ["filtering", "non-filtering", "comparison"], D = ["keep", "zero", "replace", "invert", "increment-clamp", "decrement-clamp", "increment-wrap", "decrement-wrap"], ue = ["write-only", "read-only", "read-write"], F = ["store", "discard"], j = ["all", "stencil-only", "depth-only"], fe = ["1d", "2d", "3d"], y = ["r8unorm", "r8snorm", "r8uint", "r8sint", "r16uint", "r16sint", "r16float", "rg8unorm", "rg8snorm", "rg8uint", "rg8sint", "r32uint", "r32sint", "r32float", "rg16uint", "rg16sint", "rg16float", "rgba8unorm", "rgba8unorm-srgb", "rgba8snorm", "rgba8uint", "rgba8sint", "bgra8unorm", "bgra8unorm-srgb", "rgb9e5ufloat", "rgb10a2uint", "rgb10a2unorm", "rg11b10ufloat", "rg32uint", "rg32sint", "rg32float", "rgba16uint", "rgba16sint", "rgba16float", "rgba32uint", "rgba32sint", "rgba32float", "stencil8", "depth16unorm", "depth24plus", "depth24plus-stencil8", "depth32float", "depth32float-stencil8", "bc1-rgba-unorm", "bc1-rgba-unorm-srgb", "bc2-rgba-unorm", "bc2-rgba-unorm-srgb", "bc3-rgba-unorm", "bc3-rgba-unorm-srgb", "bc4-r-unorm", "bc4-r-snorm", "bc5-rg-unorm", "bc5-rg-snorm", "bc6h-rgb-ufloat", "bc6h-rgb-float", "bc7-rgba-unorm", "bc7-rgba-unorm-srgb", "etc2-rgb8unorm", "etc2-rgb8unorm-srgb", "etc2-rgb8a1unorm", "etc2-rgb8a1unorm-srgb", "etc2-rgba8unorm", "etc2-rgba8unorm-srgb", "eac-r11unorm", "eac-r11snorm", "eac-rg11unorm", "eac-rg11snorm", "astc-4x4-unorm", "astc-4x4-unorm-srgb", "astc-5x4-unorm", "astc-5x4-unorm-srgb", "astc-5x5-unorm", "astc-5x5-unorm-srgb", "astc-6x5-unorm", "astc-6x5-unorm-srgb", "astc-6x6-unorm", "astc-6x6-unorm-srgb", "astc-8x5-unorm", "astc-8x5-unorm-srgb", "astc-8x6-unorm", "astc-8x6-unorm-srgb", "astc-8x8-unorm", "astc-8x8-unorm-srgb", "astc-10x5-unorm", "astc-10x5-unorm-srgb", "astc-10x6-unorm", "astc-10x6-unorm-srgb", "astc-10x8-unorm", "astc-10x8-unorm-srgb", "astc-10x10-unorm", "astc-10x10-unorm-srgb", "astc-12x10-unorm", "astc-12x10-unorm-srgb", "astc-12x12-unorm", "astc-12x12-unorm-srgb"], be = ["float", "unfilterable-float", "depth", "sint", "uint"], L = ["1d", "2d", "2d-array", "cube", "cube-array", "3d"], de = ["uint8", "uint8x2", "uint8x4", "sint8", "sint8x2", "sint8x4", "unorm8", "unorm8x2", "unorm8x4", "snorm8", "snorm8x2", "snorm8x4", "uint16", "uint16x2", "uint16x4", "sint16", "sint16x2", "sint16x4", "unorm16", "unorm16x2", "unorm16x4", "snorm16", "snorm16x2", "snorm16x4", "float16", "float16x2", "float16x4", "float32", "float32x2", "float32x3", "float32x4", "uint32", "uint32x2", "uint32x3", "uint32x4", "sint32", "sint32x2", "sint32x3", "sint32x4", "unorm10-10-10-2", "unorm8x4-bgra"], ge = ["vertex", "instance"], U = typeof FinalizationRegistry > "u" ? { register: () => {
}, unregister: () => {
} } : new FinalizationRegistry((o) => c.__wbg_webviewer_free(o >>> 0, 1));
function i(o) {
  A === x.length && x.push(x.length + 1);
  const e = A;
  return A = x[e], x[e] = o, e;
}
const E = typeof FinalizationRegistry > "u" ? { register: () => {
}, unregister: () => {
} } : new FinalizationRegistry((o) => o.dtor(o.a, o.b));
function R(o) {
  const e = typeof o;
  if (e == "number" || e == "boolean" || o == null)
    return `${o}`;
  if (e == "string")
    return `"${o}"`;
  if (e == "symbol") {
    const r = o.description;
    return r == null ? "Symbol" : `Symbol(${r})`;
  }
  if (e == "function") {
    const r = o.name;
    return typeof r == "string" && r.length > 0 ? `Function(${r})` : "Function";
  }
  if (Array.isArray(o)) {
    const r = o.length;
    let a = "[";
    r > 0 && (a += R(o[0]));
    for (let s = 1; s < r; s++)
      a += ", " + R(o[s]);
    return a += "]", a;
  }
  const t = /\[object ([^\]]+)\]/.exec(toString.call(o));
  let _;
  if (t && t.length > 1)
    _ = t[1];
  else
    return toString.call(o);
  if (_ == "Object")
    try {
      return "Object(" + JSON.stringify(o) + ")";
    } catch {
      return "Object";
    }
  return o instanceof Error ? `${o.name}: ${o.message}
${o.stack}` : _;
}
function we(o) {
  o < 1028 || (x[o] = A, A = o);
}
function $(o, e) {
  return o = o >>> 0, le().subarray(o / 4, o / 4 + e);
}
function k(o, e) {
  return o = o >>> 0, S().subarray(o / 1, o / 1 + e);
}
let h = null;
function w() {
  return (h === null || h.buffer.detached === !0 || h.buffer.detached === void 0 && h.buffer !== c.memory.buffer) && (h = new DataView(c.memory.buffer)), h;
}
function f(o, e) {
  return o = o >>> 0, xe(o, e);
}
let v = null;
function le() {
  return (v === null || v.byteLength === 0) && (v = new Uint32Array(c.memory.buffer)), v;
}
let B = null;
function S() {
  return (B === null || B.byteLength === 0) && (B = new Uint8Array(c.memory.buffer)), B;
}
function n(o) {
  return x[o];
}
function b(o, e) {
  try {
    return o.apply(this, e);
  } catch (t) {
    c.__wbindgen_export3(i(t));
  }
}
let x = new Array(1024).fill(void 0);
x.push(void 0, null, !0, !1);
let A = x.length;
function m(o) {
  return o == null;
}
function N(o, e, t, _) {
  const r = { a: o, b: e, cnt: 1, dtor: t }, a = (...s) => {
    r.cnt++;
    const u = r.a;
    r.a = 0;
    try {
      return _(u, r.b, ...s);
    } finally {
      r.a = u, a._wbg_cb_unref();
    }
  };
  return a._wbg_cb_unref = () => {
    --r.cnt === 0 && (r.dtor(r.a, r.b), r.a = 0, E.unregister(r));
  }, E.register(a, r, r), a;
}
function pe(o, e) {
  const t = e(o.length * 1, 1) >>> 0;
  return S().set(o, t / 1), g = o.length, t;
}
function l(o, e, t) {
  if (t === void 0) {
    const u = P.encode(o), p = e(u.length, 1) >>> 0;
    return S().subarray(p, p + u.length).set(u), g = u.length, p;
  }
  let _ = o.length, r = e(_, 1) >>> 0;
  const a = S();
  let s = 0;
  for (; s < _; s++) {
    const u = o.charCodeAt(s);
    if (u > 127) break;
    a[r + s] = u;
  }
  if (s !== _) {
    s !== 0 && (o = o.slice(s)), r = t(r, _, _ = s + o.length * 3, 1) >>> 0;
    const u = S().subarray(r + s, r + _), p = P.encodeInto(o, u);
    s += p.written, r = t(r, _, s, 1) >>> 0;
  }
  return g = s, r;
}
function d(o) {
  const e = n(o);
  return we(o), e;
}
let C = new TextDecoder("utf-8", { ignoreBOM: !0, fatal: !0 });
C.decode();
const me = 2146435072;
let z = 0;
function xe(o, e) {
  return z += e, z >= me && (C = new TextDecoder("utf-8", { ignoreBOM: !0, fatal: !0 }), C.decode(), z = e), C.decode(S().subarray(o, o + e));
}
const P = new TextEncoder();
"encodeInto" in P || (P.encodeInto = function(o, e) {
  const t = P.encode(o);
  return e.set(t), {
    read: o.length,
    written: t.length
  };
});
let g = 0, c;
function X(o, e) {
  return c = o.exports, h = null, v = null, B = null, c.__wbindgen_start(), c;
}
async function ye(o, e) {
  if (typeof Response == "function" && o instanceof Response) {
    if (typeof WebAssembly.instantiateStreaming == "function")
      try {
        return await WebAssembly.instantiateStreaming(o, e);
      } catch (r) {
        if (o.ok && t(o.type) && o.headers.get("Content-Type") !== "application/wasm")
          console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", r);
        else
          throw r;
      }
    const _ = await o.arrayBuffer();
    return await WebAssembly.instantiate(_, e);
  } else {
    const _ = await WebAssembly.instantiate(o, e);
    return _ instanceof WebAssembly.Instance ? { instance: _, module: o } : _;
  }
  function t(_) {
    switch (_) {
      case "basic":
      case "cors":
      case "default":
        return !0;
    }
    return !1;
  }
}
function Se(o) {
  if (c !== void 0) return c;
  o !== void 0 && (Object.getPrototypeOf(o) === Object.prototype ? { module: o } = o : console.warn("using deprecated parameters for `initSync()`; pass a single object instead"));
  const e = H();
  o instanceof WebAssembly.Module || (o = new WebAssembly.Module(o));
  const t = new WebAssembly.Instance(o, e);
  return X(t);
}
async function ve(o) {
  if (c !== void 0) return c;
  o !== void 0 && (Object.getPrototypeOf(o) === Object.prototype ? { module_or_path: o } = o : console.warn("using deprecated parameters for the initialization function; pass a single object instead")), o === void 0 && (o = new URL("patinae_web_bg.wasm", /* @__PURE__ */ ((r) => r)(import.meta.url)));
  const e = H();
  (typeof o == "string" || typeof Request == "function" && o instanceof Request || typeof URL == "function" && o instanceof URL) && (o = fetch(o));
  const { instance: t, module: _ } = await ye(await o, e);
  return X(t);
}
export {
  T as WebViewer,
  ve as default,
  he as init,
  Se as initSync
};
