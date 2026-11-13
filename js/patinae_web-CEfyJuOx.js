class W {
  static __wrap(e) {
    const t = Object.create(W.prototype);
    return t.__wbg_ptr = e, X.register(t, t.__wbg_ptr, t), t;
  }
  __destroy_into_raw() {
    const e = this.__wbg_ptr;
    return this.__wbg_ptr = 0, X.unregister(this), e;
  }
  free() {
    const e = this.__destroy_into_raw();
    o.__wbg_webviewer_free(e, 0);
  }
  /**
   * Cancels input after focus loss, Escape, or lost pointer capture.
   */
  cancel_pointer() {
    o.webviewer_cancel_pointer(this.__wbg_ptr);
  }
  /**
   * Request cancellation and abort the corresponding browser requests.
   * @param {string} id
   * @returns {any}
   */
  cancel_task(e) {
    try {
      const i = o.__wbindgen_add_to_stack_pointer(-16), u = l(e, o.__wbindgen_export, o.__wbindgen_export2), f = w;
      o.webviewer_cancel_task(i, this.__wbg_ptr, u, f);
      var t = s().getInt32(i + 0, !0), _ = s().getInt32(i + 4, !0), r = s().getInt32(i + 8, !0);
      if (r)
        throw b(_);
      return b(t);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Return local input paths using the shared command parser.
   * @param {string} command
   * @param {string | null} [script_path]
   * @returns {any}
   */
  command_files(e, t) {
    try {
      const p = o.__wbindgen_add_to_stack_pointer(-16), k = l(e, o.__wbindgen_export, o.__wbindgen_export2), O = w;
      var _ = m(t) ? 0 : l(t, o.__wbindgen_export, o.__wbindgen_export2), r = w;
      o.webviewer_command_files(p, this.__wbg_ptr, k, O, _, r);
      var i = s().getInt32(p + 0, !0), u = s().getInt32(p + 4, !0), f = s().getInt32(p + 8, !0);
      if (f)
        throw b(u);
      return b(i);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Configures the host platform and physical pixels per logical UI pixel.
   * @param {boolean} macos
   * @param {number} scale
   */
  configure_interaction(e, t) {
    o.webviewer_configure_interaction(this.__wbg_ptr, e, t);
  }
  /**
   * @param {string} selection
   * @returns {number}
   */
  count_atoms(e) {
    try {
      const i = o.__wbindgen_add_to_stack_pointer(-16), u = l(e, o.__wbindgen_export, o.__wbindgen_export2), f = w;
      o.webviewer_count_atoms(i, this.__wbg_ptr, u, f);
      var t = s().getInt32(i + 0, !0), _ = s().getInt32(i + 4, !0), r = s().getInt32(i + 8, !0);
      if (r)
        throw b(_);
      return t >>> 0;
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
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
    const i = l(e, o.__wbindgen_export, o.__wbindgen_export2), u = w, f = l(r, o.__wbindgen_export, o.__wbindgen_export2), p = w, k = o.webviewer_createWithMemoryProfile(i, u, t, _, f, p);
    return b(k);
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
    const r = l(e, o.__wbindgen_export, o.__wbindgen_export2), i = w, u = o.webviewer_create(r, i, t, _);
    return b(u);
  }
  /**
   * Execute a command and return its admission or synchronous result.
   * @param {string} command
   * @returns {any}
   */
  execute(e) {
    try {
      const i = o.__wbindgen_add_to_stack_pointer(-16), u = l(e, o.__wbindgen_export, o.__wbindgen_export2), f = w;
      o.webviewer_execute(i, this.__wbg_ptr, u, f);
      var t = s().getInt32(i + 0, !0), _ = s().getInt32(i + 4, !0), r = s().getInt32(i + 8, !0);
      if (r)
        throw b(_);
      return b(t);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Execute a command with addressed local file contents supplied by a client.
   *
   * `files` is an object mapping original paths to Uint8Array values.
   * @param {string} command
   * @param {any} files
   * @returns {any}
   */
  execute_with_files(e, t) {
    try {
      const u = o.__wbindgen_add_to_stack_pointer(-16), f = l(e, o.__wbindgen_export, o.__wbindgen_export2), p = w;
      o.webviewer_execute_with_files(u, this.__wbg_ptr, f, p, a(t));
      var _ = s().getInt32(u + 0, !0), r = s().getInt32(u + 4, !0), i = s().getInt32(u + 8, !0);
      if (i)
        throw b(r);
      return b(_);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * @param {string} name
   * @returns {any}
   */
  get_label_object(e) {
    const t = l(e, o.__wbindgen_export, o.__wbindgen_export2), _ = w, r = o.webviewer_get_label_object(this.__wbg_ptr, t, _);
    return b(r);
  }
  /**
   * @returns {any}
   */
  get_labels() {
    const e = o.webviewer_get_labels(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @returns {any}
   */
  get_movie_state() {
    const e = o.webviewer_get_movie_state(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @param {string} name
   * @returns {any}
   */
  get_object_info(e) {
    const t = l(e, o.__wbindgen_export, o.__wbindgen_export2), _ = w, r = o.webviewer_get_object_info(this.__wbg_ptr, t, _);
    return b(r);
  }
  /**
   * @returns {any}
   */
  get_object_infos() {
    const e = o.webviewer_get_object_infos(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @returns {any}
   */
  get_object_names() {
    const e = o.webviewer_get_object_names(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @returns {any}
   */
  get_performance_snapshot() {
    const e = o.webviewer_get_performance_snapshot(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @returns {any}
   */
  get_selection_list() {
    const e = o.webviewer_get_selection_list(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @returns {any}
   */
  get_sequence_data() {
    const e = o.webviewer_get_sequence_data(this.__wbg_ptr);
    return b(e);
  }
  /**
   * Read the authoritative retained snapshot.
   * @param {string} id
   * @returns {any}
   */
  get_task(e) {
    try {
      const i = o.__wbindgen_add_to_stack_pointer(-16), u = l(e, o.__wbindgen_export, o.__wbindgen_export2), f = w;
      o.webviewer_get_task(i, this.__wbg_ptr, u, f);
      var t = s().getInt32(i + 0, !0), _ = s().getInt32(i + 4, !0), r = s().getInt32(i + 8, !0);
      if (r)
        throw b(_);
      return b(t);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Returns highlight strengths in the projected labels' paint order.
   * @returns {Uint8Array}
   */
  label_highlights() {
    try {
      const r = o.__wbindgen_add_to_stack_pointer(-16);
      o.webviewer_label_highlights(r, this.__wbg_ptr);
      var e = s().getInt32(r + 0, !0), t = s().getInt32(r + 4, !0), _ = B(e, t).slice();
      return o.__wbindgen_export4(e, t * 1, 1), _;
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Clears hover when the pointer leaves the viewport.
   */
  leave_pointer() {
    o.webviewer_leave_pointer(this.__wbg_ptr);
  }
  /**
   * List tasks from this viewer, using the common filters and cursor.
   * @param {any} filters
   * @returns {any}
   */
  list_tasks(e) {
    try {
      const i = o.__wbindgen_add_to_stack_pointer(-16);
      o.webviewer_list_tasks(i, this.__wbg_ptr, a(e));
      var t = s().getInt32(i + 0, !0), _ = s().getInt32(i + 4, !0), r = s().getInt32(i + 8, !0);
      if (r)
        throw b(_);
      return b(t);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Apply bytes synchronously and return the common command receipt.
   * @param {Uint8Array} data
   * @param {string} name
   * @param {string} format
   * @returns {any}
   */
  load_data(e, t, _) {
    try {
      const f = o.__wbindgen_add_to_stack_pointer(-16), p = Se(e, o.__wbindgen_export), k = w, O = l(t, o.__wbindgen_export, o.__wbindgen_export2), Q = w, ee = l(_, o.__wbindgen_export, o.__wbindgen_export2), te = w;
      o.webviewer_load_data(f, this.__wbg_ptr, p, k, O, Q, ee, te);
      var r = s().getInt32(f + 0, !0), i = s().getInt32(f + 4, !0), u = s().getInt32(f + 8, !0);
      if (u)
        throw b(i);
      return b(r);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * Returns the shared controller's cursor for the canvas.
   * @returns {string}
   */
  manipulation_cursor() {
    let e, t;
    try {
      const i = o.__wbindgen_add_to_stack_pointer(-16);
      o.webviewer_manipulation_cursor(i, this.__wbg_ptr);
      var _ = s().getInt32(i + 0, !0), r = s().getInt32(i + 4, !0);
      return e = _, t = r, d(_, r);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16), o.__wbindgen_export4(e, t, 1);
    }
  }
  /**
   * @returns {boolean}
   */
  needs_redraw() {
    return o.webviewer_needs_redraw(this.__wbg_ptr) !== 0;
  }
  /**
   * Updates manipulation affordances even when the pointer is stationary.
   * @param {number} bits
   */
  on_modifiers(e) {
    o.webviewer_on_modifiers(this.__wbg_ptr, e);
  }
  /**
   * @param {number} x
   * @param {number} y
   * @param {number} button
   * @param {number} modifiers
   */
  on_mouse_down(e, t, _, r) {
    o.webviewer_on_mouse_down(this.__wbg_ptr, e, t, _, r);
  }
  /**
   * @param {number} x
   * @param {number} y
   * @param {number} modifiers
   */
  on_mouse_move(e, t, _) {
    o.webviewer_on_mouse_move(this.__wbg_ptr, e, t, _);
  }
  /**
   * @param {number} x
   * @param {number} y
   * @param {number} button
   * @returns {boolean}
   */
  on_mouse_up(e, t, _) {
    return o.webviewer_on_mouse_up(this.__wbg_ptr, e, t, _) !== 0;
  }
  /**
   * @param {number} delta_y
   * @param {number} modifiers
   */
  on_wheel(e, t) {
    o.webviewer_on_wheel(this.__wbg_ptr, e, t);
  }
  /**
   * @param {number} screen_x
   * @param {number} screen_y
   * @returns {any}
   */
  pick_at_screen(e, t) {
    const _ = o.webviewer_pick_at_screen(this.__wbg_ptr, e, t);
    return b(_);
  }
  poll_pending_picks() {
    o.webviewer_poll_pending_picks(this.__wbg_ptr);
  }
  /**
   * @param {number} screen_x
   * @param {number} screen_y
   */
  process_hover(e, t) {
    o.webviewer_process_hover(this.__wbg_ptr, e, t);
  }
  process_input() {
    o.webviewer_process_input(this.__wbg_ptr);
  }
  render_frame() {
    o.webviewer_render_frame(this.__wbg_ptr);
  }
  reset_performance_stats() {
    o.webviewer_reset_performance_stats(this.__wbg_ptr);
  }
  /**
   * @param {number} width
   * @param {number} height
   */
  resize(e, t) {
    o.webviewer_resize(this.__wbg_ptr, e, t);
  }
  /**
   * Accepts actual DOM text extents; target selection remains in shared Rust.
   * @param {any} bounds
   */
  set_label_bounds(e) {
    try {
      const r = o.__wbindgen_add_to_stack_pointer(-16);
      o.webviewer_set_label_bounds(r, this.__wbg_ptr, a(e));
      var t = s().getInt32(r + 0, !0), _ = s().getInt32(r + 4, !0);
      if (_)
        throw b(t);
    } finally {
      o.__wbindgen_add_to_stack_pointer(16);
    }
  }
  /**
   * @param {boolean} enabled
   */
  set_picking_enabled(e) {
    o.webviewer_set_picking_enabled(this.__wbg_ptr, e);
  }
  /**
   * @param {boolean} enabled
   */
  set_selection_overlay_enabled(e) {
    o.webviewer_set_selection_overlay_enabled(this.__wbg_ptr, e);
  }
  /**
   * Subscribe to task changes as hints to reread the registry.
   * @param {Function | null} [listener]
   */
  set_task_listener(e) {
    o.webviewer_set_task_listener(this.__wbg_ptr, m(e) ? 0 : a(e));
  }
  /**
   * @returns {any}
   */
  takeWarnings() {
    const e = o.webviewer_takeWarnings(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @returns {any}
   */
  take_completed_pick() {
    const e = o.webviewer_take_completed_pick(this.__wbg_ptr);
    return b(e);
  }
  /**
   * @param {number} dt
   */
  update_animations(e) {
    o.webviewer_update_animations(this.__wbg_ptr, e);
  }
  /**
   * Wait for a terminal snapshot; timeout never cancels the task.
   * @param {string} id
   * @param {number | null} [timeout_ms]
   * @returns {Promise<any>}
   */
  wait_task(e, t) {
    const _ = l(e, o.__wbindgen_export, o.__wbindgen_export2), r = w, i = o.webviewer_wait_task(this.__wbg_ptr, _, r, !m(t), m(t) ? 0 : t);
    return b(i);
  }
}
Symbol.dispose && (W.prototype[Symbol.dispose] = W.prototype.free);
function Ae() {
  o.init();
}
function Z() {
  return {
    __proto__: null,
    "./patinae_web_bg.js": {
      __proto__: null,
      __wbg_Error_30c8987f7c2ed4e2: function(e, t) {
        const _ = Error(d(e, t));
        return a(_);
      },
      __wbg_Number_14af1003b8dd5ead: function(e) {
        return Number(n(e));
      },
      __wbg_String_8564e559799eccda: function(e, t) {
        const _ = String(n(t)), r = l(_, o.__wbindgen_export, o.__wbindgen_export2), i = w;
        s().setInt32(e + 4, i, !0), s().setInt32(e + 0, r, !0);
      },
      __wbg_Window_a2a6c4d665047b14: function(e) {
        const t = n(e).Window;
        return a(t);
      },
      __wbg_WorkerGlobalScope_2664448a7c667d67: function(e) {
        const t = n(e).WorkerGlobalScope;
        return a(t);
      },
      __wbg___wbindgen_bigint_get_as_i64_a2383202b9353e4c: function(e, t) {
        const _ = n(t), r = typeof _ == "bigint" ? _ : void 0;
        s().setBigInt64(e + 8, m(r) ? BigInt(0) : r, !0), s().setInt32(e + 0, !m(r), !0);
      },
      __wbg___wbindgen_boolean_get_5b446f51afd21013: function(e) {
        const t = n(e), _ = typeof t == "boolean" ? t : void 0;
        return m(_) ? 16777215 : _ ? 1 : 0;
      },
      __wbg___wbindgen_debug_string_4687d8d8c2017d52: function(e, t) {
        const _ = q(n(t)), r = l(_, o.__wbindgen_export, o.__wbindgen_export2), i = w;
        s().setInt32(e + 4, i, !0), s().setInt32(e + 0, r, !0);
      },
      __wbg___wbindgen_in_92f62ee1427d9e49: function(e, t) {
        return n(e) in n(t);
      },
      __wbg___wbindgen_is_bigint_b123553bed3bb382: function(e) {
        return typeof n(e) == "bigint";
      },
      __wbg___wbindgen_is_function_1f9d30630b8b1d3d: function(e) {
        return typeof n(e) == "function";
      },
      __wbg___wbindgen_is_null_e343b7d08827ba72: function(e) {
        return n(e) === null;
      },
      __wbg___wbindgen_is_object_3c45d4f2dde4e749: function(e) {
        const t = n(e);
        return typeof t == "object" && t !== null;
      },
      __wbg___wbindgen_is_string_90b56bc79aad6f6c: function(e) {
        return typeof n(e) == "string";
      },
      __wbg___wbindgen_is_undefined_8865fb403f8fe9d8: function(e) {
        return n(e) === void 0;
      },
      __wbg___wbindgen_jsval_eq_02babf21faa37971: function(e, t) {
        return n(e) === n(t);
      },
      __wbg___wbindgen_jsval_loose_eq_677f21e468d6b461: function(e, t) {
        return n(e) == n(t);
      },
      __wbg___wbindgen_number_get_2e0e7dee9f701a71: function(e, t) {
        const _ = n(t), r = typeof _ == "number" ? _ : void 0;
        s().setFloat64(e + 8, m(r) ? 0 : r, !0), s().setInt32(e + 0, !m(r), !0);
      },
      __wbg___wbindgen_string_get_0380ccaa2f57f0d9: function(e, t) {
        const _ = n(t), r = typeof _ == "string" ? _ : void 0;
        var i = m(r) ? 0 : l(r, o.__wbindgen_export, o.__wbindgen_export2), u = w;
        s().setInt32(e + 4, u, !0), s().setInt32(e + 0, i, !0);
      },
      __wbg___wbindgen_throw_41e9ee4f547fc59a: function(e, t) {
        throw new Error(d(e, t));
      },
      __wbg__wbg_cb_unref_dcc1a90847f04c41: function(e) {
        n(e)._wbg_cb_unref();
      },
      __wbg_abort_f78b99d80248bacb: function(e) {
        n(e).abort();
      },
      __wbg_aborted_5b2b5f06b5e59fde: function(e) {
        return n(e).aborted;
      },
      __wbg_arrayBuffer_f07be74819944ea2: function() {
        return g(function(e) {
          const t = n(e).arrayBuffer();
          return a(t);
        }, arguments);
      },
      __wbg_beginComputePass_b9a325184985e6ff: function(e, t) {
        const _ = n(e).beginComputePass(n(t));
        return a(_);
      },
      __wbg_beginRenderPass_3c53642423af50dc: function() {
        return g(function(e, t) {
          const _ = n(e).beginRenderPass(n(t));
          return a(_);
        }, arguments);
      },
      __wbg_call_187d372bd5fdd4aa: function() {
        return g(function(e, t, _) {
          const r = n(e).call(n(t), n(_));
          return a(r);
        }, arguments);
      },
      __wbg_call_6137034ef55c9d0f: function() {
        return g(function(e, t) {
          const _ = n(e).call(n(t));
          return a(_);
        }, arguments);
      },
      __wbg_clearBuffer_3d5a3057c3c8a9bf: function(e, t, _, r) {
        n(e).clearBuffer(n(t), _, r);
      },
      __wbg_clearBuffer_82ba999c294f42fb: function(e, t, _) {
        n(e).clearBuffer(n(t), _);
      },
      __wbg_configure_1e2c1c9edad07d26: function() {
        return g(function(e, t) {
          n(e).configure(n(t));
        }, arguments);
      },
      __wbg_copyBufferToBuffer_01766818654a9868: function() {
        return g(function(e, t, _, r, i) {
          n(e).copyBufferToBuffer(n(t), _, n(r), i);
        }, arguments);
      },
      __wbg_copyBufferToBuffer_9c174b96fb08d551: function() {
        return g(function(e, t, _, r, i, u) {
          n(e).copyBufferToBuffer(n(t), _, n(r), i, u);
        }, arguments);
      },
      __wbg_copyTextureToBuffer_1234b3210431ad05: function() {
        return g(function(e, t, _, r) {
          n(e).copyTextureToBuffer(n(t), n(_), n(r));
        }, arguments);
      },
      __wbg_copyTextureToTexture_d2e6a1eb3254b828: function() {
        return g(function(e, t, _, r) {
          n(e).copyTextureToTexture(n(t), n(_), n(r));
        }, arguments);
      },
      __wbg_createBindGroupLayout_b1bd63b4e88459d8: function() {
        return g(function(e, t) {
          const _ = n(e).createBindGroupLayout(n(t));
          return a(_);
        }, arguments);
      },
      __wbg_createBindGroup_f539b26ca341308f: function(e, t) {
        const _ = n(e).createBindGroup(n(t));
        return a(_);
      },
      __wbg_createBuffer_d800e9b1d41b2ee5: function() {
        return g(function(e, t) {
          const _ = n(e).createBuffer(n(t));
          return a(_);
        }, arguments);
      },
      __wbg_createCommandEncoder_3352d1ffc36c6fc0: function(e, t) {
        const _ = n(e).createCommandEncoder(n(t));
        return a(_);
      },
      __wbg_createComputePipeline_224fa2618d9948a0: function(e, t) {
        const _ = n(e).createComputePipeline(n(t));
        return a(_);
      },
      __wbg_createPipelineLayout_6eab52c327118937: function(e, t) {
        const _ = n(e).createPipelineLayout(n(t));
        return a(_);
      },
      __wbg_createRenderPipeline_0ebb7ebc653e9207: function() {
        return g(function(e, t) {
          const _ = n(e).createRenderPipeline(n(t));
          return a(_);
        }, arguments);
      },
      __wbg_createSampler_9bd91d7e928c0060: function(e, t) {
        const _ = n(e).createSampler(n(t));
        return a(_);
      },
      __wbg_createShaderModule_cefa51336cb288ae: function(e, t) {
        const _ = n(e).createShaderModule(n(t));
        return a(_);
      },
      __wbg_createTexture_ed7e9fc04dd54d84: function() {
        return g(function(e, t) {
          const _ = n(e).createTexture(n(t));
          return a(_);
        }, arguments);
      },
      __wbg_createView_da41c2d2cb212715: function() {
        return g(function(e, t) {
          const _ = n(e).createView(n(t));
          return a(_);
        }, arguments);
      },
      __wbg_debug_52bda05ddf50b736: function(e) {
        console.debug(n(e));
      },
      __wbg_description_83b8a393160021b9: function(e, t) {
        const _ = n(t).description, r = l(_, o.__wbindgen_export, o.__wbindgen_export2), i = w;
        s().setInt32(e + 4, i, !0), s().setInt32(e + 0, r, !0);
      },
      __wbg_dispatchWorkgroups_56b943172790add0: function(e, t, _, r) {
        n(e).dispatchWorkgroups(t >>> 0, _ >>> 0, r >>> 0);
      },
      __wbg_document_9854e03c05fc8834: function(e) {
        const t = n(e).document;
        return m(t) ? 0 : a(t);
      },
      __wbg_done_b41a1d26cdb37fb6: function(e) {
        return n(e).done;
      },
      __wbg_drawIndexed_638959aae942557c: function(e, t, _, r, i, u) {
        n(e).drawIndexed(t >>> 0, _ >>> 0, r >>> 0, i, u >>> 0);
      },
      __wbg_drawIndirect_269c8a4880a070c8: function(e, t, _) {
        n(e).drawIndirect(n(t), _);
      },
      __wbg_draw_086a9578fc9898c2: function(e, t, _, r, i) {
        n(e).draw(t >>> 0, _ >>> 0, r >>> 0, i >>> 0);
      },
      __wbg_end_82fd0c12ba185009: function(e) {
        n(e).end();
      },
      __wbg_end_b57473834b877409: function(e) {
        n(e).end();
      },
      __wbg_error_757e9472f8410341: function(e, t) {
        let _, r;
        try {
          _ = e, r = t, console.error(d(e, t));
        } finally {
          o.__wbindgen_export4(_, r, 1);
        }
      },
      __wbg_error_c9cf3fc2064683a9: function(e) {
        console.error(n(e));
      },
      __wbg_fetch_b322877f1ed6457d: function(e, t, _, r) {
        const i = n(e).fetch(d(t, _), n(r));
        return a(i);
      },
      __wbg_finish_09ec094c10f41e7b: function(e) {
        const t = n(e).finish();
        return a(t);
      },
      __wbg_finish_ec1c191f66a895b1: function(e, t) {
        const _ = n(e).finish(n(t));
        return a(_);
      },
      __wbg_getContext_635e36719cad2623: function() {
        return g(function(e, t, _) {
          const r = n(e).getContext(d(t, _));
          return m(r) ? 0 : a(r);
        }, arguments);
      },
      __wbg_getContext_e567868594d2e0c6: function() {
        return g(function(e, t, _) {
          const r = n(e).getContext(d(t, _));
          return m(r) ? 0 : a(r);
        }, arguments);
      },
      __wbg_getCurrentTexture_9f3b84d0eaa6cd95: function() {
        return g(function(e) {
          const t = n(e).getCurrentTexture();
          return a(t);
        }, arguments);
      },
      __wbg_getElementById_cc94972b404e4eaa: function(e, t, _) {
        const r = n(e).getElementById(d(t, _));
        return m(r) ? 0 : a(r);
      },
      __wbg_getMappedRange_fb54c6327b2d8d20: function() {
        return g(function(e, t, _) {
          const r = n(e).getMappedRange(t, _);
          return a(r);
        }, arguments);
      },
      __wbg_getPreferredCanvasFormat_0ef5034c8902201b: function(e) {
        const t = n(e).getPreferredCanvasFormat();
        return (h.indexOf(t) + 1 || 102) - 1;
      },
      __wbg_getRandomValues_a608c4436c19407a: function() {
        return g(function(e, t) {
          globalThis.crypto.getRandomValues(B(e, t));
        }, arguments);
      },
      __wbg_get_31af05bd4842a84f: function() {
        return g(function(e, t) {
          const _ = Reflect.get(n(e), n(t));
          return a(_);
        }, arguments);
      },
      __wbg_get_658f6698067d9515: function() {
        return g(function(e, t) {
          const _ = Reflect.get(n(e), n(t));
          return a(_);
        }, arguments);
      },
      __wbg_get_6c896e0571ddae51: function(e, t) {
        const _ = n(e)[t >>> 0];
        return a(_);
      },
      __wbg_get_fd12a9100976c296: function(e, t) {
        const _ = n(e)[t >>> 0];
        return m(_) ? 0 : a(_);
      },
      __wbg_get_unchecked_288889d017702237: function(e, t) {
        const _ = n(e)[t >>> 0];
        return a(_);
      },
      __wbg_get_with_ref_key_6412cf3094599694: function(e, t) {
        const _ = n(e)[n(t)];
        return a(_);
      },
      __wbg_gpu_afdd4387c7afe5f9: function(e) {
        const t = n(e).gpu;
        return a(t);
      },
      __wbg_height_fc97e1a0c2e7331f: function(e) {
        return n(e).height;
      },
      __wbg_href_221703ee1ed727fc: function(e, t) {
        const _ = n(t).href, r = l(_, o.__wbindgen_export, o.__wbindgen_export2), i = w;
        s().setInt32(e + 4, i, !0), s().setInt32(e + 0, r, !0);
      },
      __wbg_info_26925f73a2eee895: function(e) {
        console.info(n(e));
      },
      __wbg_info_971d8b9db3dae69f: function(e) {
        const t = n(e).info;
        return a(t);
      },
      __wbg_instanceof_ArrayBuffer_a99f175873e5d9b8: function(e) {
        let t;
        try {
          t = n(e) instanceof ArrayBuffer;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_instanceof_HtmlCanvasElement_0a30c11fbbf41841: function(e) {
        let t;
        try {
          t = n(e) instanceof HTMLCanvasElement;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_instanceof_Response_b8758567269c30b2: function(e) {
        let t;
        try {
          t = n(e) instanceof Response;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_instanceof_Uint8Array_828cef2aaacafc31: function(e) {
        let t;
        try {
          t = n(e) instanceof Uint8Array;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_instanceof_Window_82d71df4eddf88bc: function(e) {
        let t;
        try {
          t = n(e) instanceof Window;
        } catch {
          t = !1;
        }
        return t;
      },
      __wbg_isArray_e15a2ff68ffdbef2: function(e) {
        return Array.isArray(n(e));
      },
      __wbg_isFallbackAdapter_4c8cc3b18677460a: function(e) {
        return n(e).isFallbackAdapter;
      },
      __wbg_isSafeInteger_717808ad6a54bd9e: function(e) {
        return Number.isSafeInteger(n(e));
      },
      __wbg_iterator_e3c31c892080e444: function() {
        return a(Symbol.iterator);
      },
      __wbg_keys_440be172f17c0265: function(e) {
        const t = Object.keys(n(e));
        return a(t);
      },
      __wbg_label_7add8cb37a6ef98f: function(e, t) {
        const _ = n(t).label, r = l(_, o.__wbindgen_export, o.__wbindgen_export2), i = w;
        s().setInt32(e + 4, i, !0), s().setInt32(e + 0, r, !0);
      },
      __wbg_length_7f3c00c40364105e: function(e) {
        return n(e).length;
      },
      __wbg_length_d4bdea10311bd9cf: function(e) {
        return n(e).length;
      },
      __wbg_limits_06bcb36c8409843b: function(e) {
        const t = n(e).limits;
        return a(t);
      },
      __wbg_limits_601ad2e086ef8141: function(e) {
        const t = n(e).limits;
        return a(t);
      },
      __wbg_log_17c30ef363c61cf4: function(e) {
        console.log(n(e));
      },
      __wbg_mapAsync_b0597127f5037286: function(e, t, _, r) {
        const i = n(e).mapAsync(t >>> 0, _, r);
        return a(i);
      },
      __wbg_maxBindGroupsPlusVertexBuffers_52369f089736ef9d: function(e) {
        return n(e).maxBindGroupsPlusVertexBuffers;
      },
      __wbg_maxBindGroups_4e424afe6ce86ca2: function(e) {
        return n(e).maxBindGroups;
      },
      __wbg_maxBindingsPerBindGroup_7d035da36821c44f: function(e) {
        return n(e).maxBindingsPerBindGroup;
      },
      __wbg_maxBufferSize_423f4a084e32a195: function(e) {
        return n(e).maxBufferSize;
      },
      __wbg_maxColorAttachmentBytesPerSample_c4cd9126f6d287c6: function(e) {
        return n(e).maxColorAttachmentBytesPerSample;
      },
      __wbg_maxColorAttachments_d924670762b9e250: function(e) {
        return n(e).maxColorAttachments;
      },
      __wbg_maxComputeInvocationsPerWorkgroup_707a3868f7cebb59: function(e) {
        return n(e).maxComputeInvocationsPerWorkgroup;
      },
      __wbg_maxComputeWorkgroupSizeX_0a4d99463cbd6e5e: function(e) {
        return n(e).maxComputeWorkgroupSizeX;
      },
      __wbg_maxComputeWorkgroupSizeY_85123ea0587f7558: function(e) {
        return n(e).maxComputeWorkgroupSizeY;
      },
      __wbg_maxComputeWorkgroupSizeZ_a3186b4c5267d44f: function(e) {
        return n(e).maxComputeWorkgroupSizeZ;
      },
      __wbg_maxComputeWorkgroupStorageSize_57b297355cfb6204: function(e) {
        return n(e).maxComputeWorkgroupStorageSize;
      },
      __wbg_maxComputeWorkgroupsPerDimension_4158f95e673d54c4: function(e) {
        return n(e).maxComputeWorkgroupsPerDimension;
      },
      __wbg_maxDynamicStorageBuffersPerPipelineLayout_226b0b70910aa16c: function(e) {
        return n(e).maxDynamicStorageBuffersPerPipelineLayout;
      },
      __wbg_maxDynamicUniformBuffersPerPipelineLayout_0e835fda711fc7e6: function(e) {
        return n(e).maxDynamicUniformBuffersPerPipelineLayout;
      },
      __wbg_maxInterStageShaderVariables_8c4a1d727e2aa35a: function(e) {
        return n(e).maxInterStageShaderVariables;
      },
      __wbg_maxSampledTexturesPerShaderStage_6675f5e91d9a728a: function(e) {
        return n(e).maxSampledTexturesPerShaderStage;
      },
      __wbg_maxSamplersPerShaderStage_1910fa38a6ed1e1f: function(e) {
        return n(e).maxSamplersPerShaderStage;
      },
      __wbg_maxStorageBufferBindingSize_2e244bded070b18d: function(e) {
        return n(e).maxStorageBufferBindingSize;
      },
      __wbg_maxStorageBuffersPerShaderStage_a285f3ebca51ca0d: function(e) {
        return n(e).maxStorageBuffersPerShaderStage;
      },
      __wbg_maxStorageTexturesPerShaderStage_7aa946f0fc322a2b: function(e) {
        return n(e).maxStorageTexturesPerShaderStage;
      },
      __wbg_maxTextureArrayLayers_0e699147ad00502d: function(e) {
        return n(e).maxTextureArrayLayers;
      },
      __wbg_maxTextureDimension1D_aabf6add54decfe2: function(e) {
        return n(e).maxTextureDimension1D;
      },
      __wbg_maxTextureDimension2D_dd598b27e9c0c1c4: function(e) {
        return n(e).maxTextureDimension2D;
      },
      __wbg_maxTextureDimension3D_f944266c65dfd1a9: function(e) {
        return n(e).maxTextureDimension3D;
      },
      __wbg_maxUniformBufferBindingSize_59fa6be7cfbeeb53: function(e) {
        return n(e).maxUniformBufferBindingSize;
      },
      __wbg_maxUniformBuffersPerShaderStage_bee5f00a4d706c7f: function(e) {
        return n(e).maxUniformBuffersPerShaderStage;
      },
      __wbg_maxVertexAttributes_5cf6392c4e9033fe: function(e) {
        return n(e).maxVertexAttributes;
      },
      __wbg_maxVertexBufferArrayStride_548baa887375d865: function(e) {
        return n(e).maxVertexBufferArrayStride;
      },
      __wbg_maxVertexBuffers_75d881156591f5da: function(e) {
        return n(e).maxVertexBuffers;
      },
      __wbg_minStorageBufferOffsetAlignment_5ba9b77792bdadb3: function(e) {
        return n(e).minStorageBufferOffsetAlignment;
      },
      __wbg_minUniformBufferOffsetAlignment_ab7d52a5293b22bd: function(e) {
        return n(e).minUniformBufferOffsetAlignment;
      },
      __wbg_navigator_2156486643462a87: function(e) {
        const t = n(e).navigator;
        return a(t);
      },
      __wbg_navigator_c9bce48d4b578c9c: function(e) {
        const t = n(e).navigator;
        return a(t);
      },
      __wbg_new_1dbf7428bba60a42: function(e) {
        const t = new Uint8Array(n(e));
        return a(t);
      },
      __wbg_new_227d7c05414eb861: function() {
        const e = new Error();
        return a(e);
      },
      __wbg_new_28744009d011f847: function() {
        return a(/* @__PURE__ */ new Map());
      },
      __wbg_new_2e41357094e99294: function() {
        return g(function() {
          const e = new AbortController();
          return a(e);
        }, arguments);
      },
      __wbg_new_5502aad30c185fc8: function(e, t) {
        try {
          var _ = { a: e, b: t }, r = (u, f) => {
            const p = _.a;
            _.a = 0;
            try {
              return U(p, _.b, u, f);
            } finally {
              _.a = p;
            }
          };
          const i = new Promise(r);
          return a(i);
        } finally {
          _.a = 0;
        }
      },
      __wbg_new_617a8cdb8bb1130e: function() {
        const e = new Object();
        return a(e);
      },
      __wbg_new_a217df4351db4b9b: function() {
        return g(function(e, t) {
          const _ = new OffscreenCanvas(e >>> 0, t >>> 0);
          return a(_);
        }, arguments);
      },
      __wbg_new_ee2291f50781bf1d: function() {
        const e = new Array();
        return a(e);
      },
      __wbg_new_typed_689a3a281a8d4da6: function() {
        const e = new Object();
        return a(e);
      },
      __wbg_new_typed_b01cb72a8af741a3: function(e, t) {
        try {
          var _ = { a: e, b: t }, r = (u, f) => {
            const p = _.a;
            _.a = 0;
            try {
              return U(p, _.b, u, f);
            } finally {
              _.a = p;
            }
          };
          const i = new Promise(r);
          return a(i);
        } finally {
          _.a = 0;
        }
      },
      __wbg_new_with_base_29cdb449ce79d2d5: function() {
        return g(function(e, t, _, r) {
          const i = new URL(d(e, t), d(_, r));
          return a(i);
        }, arguments);
      },
      __wbg_new_with_byte_offset_and_length_2f5d7fc2a828b74d: function(e, t, _) {
        const r = new Uint8Array(n(e), t >>> 0, _ >>> 0);
        return a(r);
      },
      __wbg_next_33784799010f1bbe: function(e) {
        const t = n(e).next;
        return a(t);
      },
      __wbg_next_f4aac29c42af995c: function() {
        return g(function(e) {
          const t = n(e).next();
          return a(t);
        }, arguments);
      },
      __wbg_now_aa4ccb83129e9e55: function() {
        return Date.now();
      },
      __wbg_now_d0b7f4bea9f38490: function(e) {
        return n(e).now();
      },
      __wbg_now_e7c6795a7f81e10f: function(e) {
        return n(e).now();
      },
      __wbg_ok_125878a7db256ecb: function(e) {
        return n(e).ok;
      },
      __wbg_onSubmittedWorkDone_1190213cee1ecf7e: function(e) {
        const t = n(e).onSubmittedWorkDone();
        return a(t);
      },
      __wbg_performance_3fcf6e32a7e1ed0a: function(e) {
        const t = n(e).performance;
        return a(t);
      },
      __wbg_performance_d0d7b03fc649b694: function(e) {
        const t = n(e).performance;
        return m(t) ? 0 : a(t);
      },
      __wbg_prototypesetcall_bc27214492979395: function(e, t, _) {
        Uint8Array.prototype.set.call(B(e, t), n(_));
      },
      __wbg_querySelectorAll_6aebfe3df1fb2014: function() {
        return g(function(e, t, _) {
          const r = n(e).querySelectorAll(d(t, _));
          return a(r);
        }, arguments);
      },
      __wbg_queueMicrotask_9833f9a49df95a49: function(e) {
        const t = n(e).queueMicrotask;
        return a(t);
      },
      __wbg_queueMicrotask_a72f977e97f23c5f: function(e) {
        queueMicrotask(n(e));
      },
      __wbg_queue_7b62c28143d44293: function(e) {
        const t = n(e).queue;
        return a(t);
      },
      __wbg_random_5a4cafd2f02395ff: function() {
        return Math.random();
      },
      __wbg_requestAdapter_a539af006419f2e9: function(e, t) {
        const _ = n(e).requestAdapter(n(t));
        return a(_);
      },
      __wbg_requestDevice_5cb8a582e55d08cb: function(e, t) {
        const _ = n(e).requestDevice(n(t));
        return a(_);
      },
      __wbg_resolve_0076e10020304ede: function(e) {
        const t = Promise.resolve(n(e));
        return a(t);
      },
      __wbg_setBindGroup_0b7c2a055ef1f314: function() {
        return g(function(e, t, _, r, i, u, f) {
          n(e).setBindGroup(t >>> 0, n(_), J(r, i), u, f >>> 0);
        }, arguments);
      },
      __wbg_setBindGroup_11bdbb60cc8b54b9: function() {
        return g(function(e, t, _, r, i, u, f) {
          n(e).setBindGroup(t >>> 0, n(_), J(r, i), u, f >>> 0);
        }, arguments);
      },
      __wbg_setBindGroup_418c3e0eb6943ce0: function(e, t, _) {
        n(e).setBindGroup(t >>> 0, n(_));
      },
      __wbg_setBindGroup_c83391351ce68826: function(e, t, _) {
        n(e).setBindGroup(t >>> 0, n(_));
      },
      __wbg_setIndexBuffer_01327df91742b73e: function(e, t, _, r) {
        n(e).setIndexBuffer(n(t), D[_], r);
      },
      __wbg_setIndexBuffer_241097e303986c14: function(e, t, _, r, i) {
        n(e).setIndexBuffer(n(t), D[_], r, i);
      },
      __wbg_setPipeline_5146f7b8d2b4af5c: function(e, t) {
        n(e).setPipeline(n(t));
      },
      __wbg_setPipeline_b6f981027e02cd16: function(e, t) {
        n(e).setPipeline(n(t));
      },
      __wbg_setScissorRect_889235eeb784732b: function(e, t, _, r, i) {
        n(e).setScissorRect(t >>> 0, _ >>> 0, r >>> 0, i >>> 0);
      },
      __wbg_setTimeout_db7bbc18a17e152a: function() {
        return g(function(e, t, _) {
          return n(e).setTimeout(n(t), _);
        }, arguments);
      },
      __wbg_setVertexBuffer_6db3b60e99280744: function(e, t, _, r) {
        n(e).setVertexBuffer(t >>> 0, n(_), r);
      },
      __wbg_setVertexBuffer_cbf4ca1627c02f4c: function(e, t, _, r, i) {
        n(e).setVertexBuffer(t >>> 0, n(_), r, i);
      },
      __wbg_setViewport_c0be08dcc8965ccf: function(e, t, _, r, i, u, f) {
        n(e).setViewport(t, _, r, i, u, f);
      },
      __wbg_set_145a351398b48c65: function() {
        return g(function(e, t, _) {
          return Reflect.set(n(e), n(t), n(_));
        }, arguments);
      },
      __wbg_set_34d08fd992d43d61: function(e, t, _) {
        n(e).set(n(t), _ >>> 0);
      },
      __wbg_set_6ae97e73113c4f0b: function(e, t, _) {
        const r = n(e).set(n(t), n(_));
        return a(r);
      },
      __wbg_set_6be42768c690e380: function(e, t, _) {
        n(e)[b(t)] = b(_);
      },
      __wbg_set_a_82818effc94f6256: function(e, t) {
        n(e).a = t;
      },
      __wbg_set_access_a099cfbbeec9b96f: function(e, t) {
        n(e).access = le[t];
      },
      __wbg_set_address_mode_u_a68737cf5d288f95: function(e, t) {
        n(e).addressModeU = G[t];
      },
      __wbg_set_address_mode_v_b1c3c45933f540d1: function(e, t) {
        n(e).addressModeV = G[t];
      },
      __wbg_set_address_mode_w_889c31cf7022c764: function(e, t) {
        n(e).addressModeW = G[t];
      },
      __wbg_set_alpha_106f21a936a85eba: function(e, t) {
        n(e).alpha = n(t);
      },
      __wbg_set_alpha_mode_5544568dbac50280: function(e, t) {
        n(e).alphaMode = ae[t];
      },
      __wbg_set_alpha_to_coverage_enabled_3372ce329447b8f1: function(e, t) {
        n(e).alphaToCoverageEnabled = t !== 0;
      },
      __wbg_set_array_layer_count_22afa0a979e4ad55: function(e, t) {
        n(e).arrayLayerCount = t >>> 0;
      },
      __wbg_set_array_stride_f64_6816040e5e7598c3: function(e, t) {
        n(e).arrayStride = t;
      },
      __wbg_set_aspect_a48d046965270281: function(e, t) {
        n(e).aspect = H[t];
      },
      __wbg_set_aspect_b1a9909bf315433f: function(e, t) {
        n(e).aspect = H[t];
      },
      __wbg_set_attributes_9e38cb1dde387a5b: function(e, t, _) {
        n(e).attributes = x(t, _);
      },
      __wbg_set_b_a3297ee7e7cac3a8: function(e, t) {
        n(e).b = t;
      },
      __wbg_set_base_array_layer_2435ba92c80346ae: function(e, t) {
        n(e).baseArrayLayer = t >>> 0;
      },
      __wbg_set_base_mip_level_8b6093e875e7c65d: function(e, t) {
        n(e).baseMipLevel = t >>> 0;
      },
      __wbg_set_bea140a88be9b277: function(e, t, _) {
        n(e)[t >>> 0] = b(_);
      },
      __wbg_set_beginning_of_pass_write_index_e552c5e8b8bbf52f: function(e, t) {
        n(e).beginningOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_beginning_of_pass_write_index_fa9ae10d5d2804ce: function(e, t) {
        n(e).beginningOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_bind_group_layouts_458c44ba55100b82: function(e, t, _) {
        n(e).bindGroupLayouts = x(t, _);
      },
      __wbg_set_binding_81b3fac7f7acaf8d: function(e, t) {
        n(e).binding = t >>> 0;
      },
      __wbg_set_binding_b6cee57f35ac5190: function(e, t) {
        n(e).binding = t >>> 0;
      },
      __wbg_set_blend_1a801617945f7945: function(e, t) {
        n(e).blend = n(t);
      },
      __wbg_set_buffer_1548ae88a9188037: function(e, t) {
        n(e).buffer = n(t);
      },
      __wbg_set_buffer_8d0ac64ad20dfc84: function(e, t) {
        n(e).buffer = n(t);
      },
      __wbg_set_buffer_910a40a90f97cfca: function(e, t) {
        n(e).buffer = n(t);
      },
      __wbg_set_buffers_5d0e0c50791f710e: function(e, t, _) {
        n(e).buffers = x(t, _);
      },
      __wbg_set_bytes_per_row_1e824a5502b54b3d: function(e, t) {
        n(e).bytesPerRow = t >>> 0;
      },
      __wbg_set_clear_value_gpu_color_dict_a9f763e8372ac1de: function(e, t) {
        n(e).clearValue = n(t);
      },
      __wbg_set_code_5d5b0b9e2fd0dca7: function(e, t, _) {
        n(e).code = d(t, _);
      },
      __wbg_set_color_8ecace4011f47d2e: function(e, t) {
        n(e).color = n(t);
      },
      __wbg_set_color_attachments_622fe2d5997fda7a: function(e, t, _) {
        n(e).colorAttachments = x(t, _);
      },
      __wbg_set_compare_080c9e492ff36990: function(e, t) {
        n(e).compare = F[t];
      },
      __wbg_set_compare_817cf3695599eaa6: function(e, t) {
        n(e).compare = F[t];
      },
      __wbg_set_compute_bc754d9e37f6e4c9: function(e, t) {
        n(e).compute = n(t);
      },
      __wbg_set_count_8ff0c9474e39a849: function(e, t) {
        n(e).count = t >>> 0;
      },
      __wbg_set_cull_mode_85d2b4ab0ce3a564: function(e, t) {
        n(e).cullMode = ue[t];
      },
      __wbg_set_depth_bias_95abf479cae3f3cd: function(e, t) {
        n(e).depthBias = t;
      },
      __wbg_set_depth_bias_clamp_ba3d0b8348151350: function(e, t) {
        n(e).depthBiasClamp = t;
      },
      __wbg_set_depth_bias_slope_scale_6b2584d93f5b9cd2: function(e, t) {
        n(e).depthBiasSlopeScale = t;
      },
      __wbg_set_depth_clear_value_e30a4c754c6b3b26: function(e, t) {
        n(e).depthClearValue = t;
      },
      __wbg_set_depth_compare_a90de4e3714397ab: function(e, t) {
        n(e).depthCompare = F[t];
      },
      __wbg_set_depth_fail_op_b5c64541d1b6b482: function(e, t) {
        n(e).depthFailOp = L[t];
      },
      __wbg_set_depth_load_op_932888016d762d3e: function(e, t) {
        n(e).depthLoadOp = z[t];
      },
      __wbg_set_depth_or_array_layers_e2f074a0284e4806: function(e, t) {
        n(e).depthOrArrayLayers = t >>> 0;
      },
      __wbg_set_depth_read_only_be790175a1c2db9a: function(e, t) {
        n(e).depthReadOnly = t !== 0;
      },
      __wbg_set_depth_stencil_attachment_54a8922f5fbe08bf: function(e, t) {
        n(e).depthStencilAttachment = n(t);
      },
      __wbg_set_depth_stencil_b7cffc59ad4da529: function(e, t) {
        n(e).depthStencil = n(t);
      },
      __wbg_set_depth_store_op_9054814f164ab55d: function(e, t) {
        n(e).depthStoreOp = R[t];
      },
      __wbg_set_depth_write_enabled_31a821ee1fb3b0b3: function(e, t) {
        n(e).depthWriteEnabled = t !== 0;
      },
      __wbg_set_device_210484a77b675c9c: function(e, t) {
        n(e).device = n(t);
      },
      __wbg_set_dimension_3da9d03131a9f446: function(e, t) {
        n(e).dimension = pe[t];
      },
      __wbg_set_dimension_56332450afa3e0c0: function(e, t) {
        n(e).dimension = j[t];
      },
      __wbg_set_dst_factor_865ba9aaf187890c: function(e, t) {
        n(e).dstFactor = $[t];
      },
      __wbg_set_end_of_pass_write_index_8f164f9e60d4ad16: function(e, t) {
        n(e).endOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_end_of_pass_write_index_b54a8e3802b9ae0d: function(e, t) {
        n(e).endOfPassWriteIndex = t >>> 0;
      },
      __wbg_set_entries_6f866302103b81e9: function(e, t, _) {
        n(e).entries = x(t, _);
      },
      __wbg_set_entries_f26b77ab9548e906: function(e, t, _) {
        n(e).entries = x(t, _);
      },
      __wbg_set_entry_point_71cef95c137b5774: function(e, t, _) {
        n(e).entryPoint = d(t, _);
      },
      __wbg_set_entry_point_ac8db535971761fe: function(e, t, _) {
        n(e).entryPoint = d(t, _);
      },
      __wbg_set_entry_point_b70f98f5025a114d: function(e, t, _) {
        n(e).entryPoint = d(t, _);
      },
      __wbg_set_external_texture_7f966c604c4f8098: function(e, t) {
        n(e).externalTexture = n(t);
      },
      __wbg_set_fail_op_d59d0187e4111dfe: function(e, t) {
        n(e).failOp = L[t];
      },
      __wbg_set_format_23f7f32549751d43: function(e, t) {
        n(e).format = h[t];
      },
      __wbg_set_format_283dca56552f07a3: function(e, t) {
        n(e).format = h[t];
      },
      __wbg_set_format_5080a858117ad2c1: function(e, t) {
        n(e).format = xe[t];
      },
      __wbg_set_format_66735b94bd868ba2: function(e, t) {
        n(e).format = h[t];
      },
      __wbg_set_format_7f2bdbfb101b1ae1: function(e, t) {
        n(e).format = h[t];
      },
      __wbg_set_format_92732ea75d3b79f5: function(e, t) {
        n(e).format = h[t];
      },
      __wbg_set_format_f009e603f7d4c28e: function(e, t) {
        n(e).format = h[t];
      },
      __wbg_set_fragment_d2b0ec97d7cf8d47: function(e, t) {
        n(e).fragment = n(t);
      },
      __wbg_set_front_face_d3f8a2e07e7b25dd: function(e, t) {
        n(e).frontFace = fe[t];
      },
      __wbg_set_g_b527ee8a9bed553d: function(e, t) {
        n(e).g = t;
      },
      __wbg_set_has_dynamic_offset_0c72ffa900c5a269: function(e, t) {
        n(e).hasDynamicOffset = t !== 0;
      },
      __wbg_set_height_c9789c1c77eaedff: function(e, t) {
        n(e).height = t >>> 0;
      },
      __wbg_set_height_f6619158e5735877: function(e, t) {
        n(e).height = t >>> 0;
      },
      __wbg_set_height_fde391767df1ce27: function(e, t) {
        n(e).height = t >>> 0;
      },
      __wbg_set_label_09e2da5f8522dbe8: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_17202740051e9722: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_17b4858e1eef23dd: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_2fefb39c0e0dbbe8: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_3cb2322e6f6db14c: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_3f2ccaafef5ff7c9: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_612add98a4398f92: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_70a09ee68d6b1b26: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_92cd3811e96b487c: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_9c2a186152427ee0: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_c3eaf136aa464cba: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_c7987704d29f284b: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_cfe64bca8945ee30: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_e02179cf97e95763: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_label_ee172cd5f6a96961: function(e, t, _) {
        n(e).label = d(t, _);
      },
      __wbg_set_layout_454e3a091b390cd4: function(e, t) {
        n(e).layout = n(t);
      },
      __wbg_set_layout_4b5d5b12fbb72a76: function(e, t) {
        n(e).layout = n(t);
      },
      __wbg_set_layout_75dc1ca3f2421cff: function(e, t) {
        n(e).layout = n(t);
      },
      __wbg_set_layout_gpu_auto_layout_mode_06a2b95af1043098: function(e, t) {
        n(e).layout = E[t];
      },
      __wbg_set_layout_gpu_auto_layout_mode_ee432e515c357fa8: function(e, t) {
        n(e).layout = E[t];
      },
      __wbg_set_load_op_c56b1269acc2d51f: function(e, t) {
        n(e).loadOp = z[t];
      },
      __wbg_set_lod_max_clamp_db24179f67f3aa31: function(e, t) {
        n(e).lodMaxClamp = t;
      },
      __wbg_set_lod_min_clamp_2bbce566e9fefa04: function(e, t) {
        n(e).lodMinClamp = t;
      },
      __wbg_set_mag_filter_db8e6b42d4f8846d: function(e, t) {
        n(e).magFilter = N[t];
      },
      __wbg_set_mapped_at_creation_3f320fef6761b02c: function(e, t) {
        n(e).mappedAtCreation = t !== 0;
      },
      __wbg_set_mask_c1079e551ec360dc: function(e, t) {
        n(e).mask = t >>> 0;
      },
      __wbg_set_max_anisotropy_84749fdcec362dc4: function(e, t) {
        n(e).maxAnisotropy = t;
      },
      __wbg_set_min_binding_size_f64_897e3cd4496ddec9: function(e, t) {
        n(e).minBindingSize = t;
      },
      __wbg_set_min_filter_d435bbfc5a637757: function(e, t) {
        n(e).minFilter = N[t];
      },
      __wbg_set_mip_level_count_047936c630acee7b: function(e, t) {
        n(e).mipLevelCount = t >>> 0;
      },
      __wbg_set_mip_level_count_44bc46a1ae6f6daa: function(e, t) {
        n(e).mipLevelCount = t >>> 0;
      },
      __wbg_set_mip_level_f3745730372683d5: function(e, t) {
        n(e).mipLevel = t >>> 0;
      },
      __wbg_set_mipmap_filter_62fb49a84b0747ff: function(e, t) {
        n(e).mipmapFilter = be[t];
      },
      __wbg_set_mode_7edfbc344ef9c650: function(e, t) {
        n(e).mode = se[t];
      },
      __wbg_set_module_392eeaa269f203b0: function(e, t) {
        n(e).module = n(t);
      },
      __wbg_set_module_715d37652c4998ec: function(e, t) {
        n(e).module = n(t);
      },
      __wbg_set_module_c9946af218d23b53: function(e, t) {
        n(e).module = n(t);
      },
      __wbg_set_multisample_ff72a7a5456cbeb7: function(e, t) {
        n(e).multisample = n(t);
      },
      __wbg_set_multisampled_039f032dc4b67367: function(e, t) {
        n(e).multisampled = t !== 0;
      },
      __wbg_set_offset_f64_127e8a0aa5c5485a: function(e, t) {
        n(e).offset = t;
      },
      __wbg_set_offset_f64_457756429ede426d: function(e, t) {
        n(e).offset = t;
      },
      __wbg_set_offset_f64_d1d115dd438165b5: function(e, t) {
        n(e).offset = t;
      },
      __wbg_set_operation_00a77386523b88f9: function(e, t) {
        n(e).operation = ce[t];
      },
      __wbg_set_origin_gpu_origin_3d_dict_0619d4860adb4eb6: function(e, t) {
        n(e).origin = n(t);
      },
      __wbg_set_pass_op_3cf10feb3d76ab97: function(e, t) {
        n(e).passOp = L[t];
      },
      __wbg_set_power_preference_b42d00a8facfbade: function(e, t) {
        n(e).powerPreference = ge[t];
      },
      __wbg_set_primitive_e796cf76f0ff89f3: function(e, t) {
        n(e).primitive = n(t);
      },
      __wbg_set_query_set_f030702f1b69199f: function(e, t) {
        n(e).querySet = n(t);
      },
      __wbg_set_query_set_f9a94586851fdba2: function(e, t) {
        n(e).querySet = n(t);
      },
      __wbg_set_r_6ece4d74af63364f: function(e, t) {
        n(e).r = t;
      },
      __wbg_set_required_features_bbab71414c45e621: function(e, t, _) {
        n(e).requiredFeatures = x(t, _);
      },
      __wbg_set_required_limits_837f62d865e7cfac: function(e, t) {
        n(e).requiredLimits = n(t);
      },
      __wbg_set_resolve_target_gpu_texture_view_e4c1e3bbb8c27d87: function(e, t) {
        n(e).resolveTarget = n(t);
      },
      __wbg_set_resource_8fd8658b30d86ecf: function(e, t) {
        n(e).resource = n(t);
      },
      __wbg_set_resource_gpu_buffer_binding_33099b25da65b610: function(e, t) {
        n(e).resource = n(t);
      },
      __wbg_set_resource_gpu_texture_view_4cffe7bc7c8e5cbe: function(e, t) {
        n(e).resource = n(t);
      },
      __wbg_set_rows_per_image_deb456502f23c260: function(e, t) {
        n(e).rowsPerImage = t >>> 0;
      },
      __wbg_set_sample_count_481c255a12054e1d: function(e, t) {
        n(e).sampleCount = t >>> 0;
      },
      __wbg_set_sample_type_ebc5fcd029513bda: function(e, t) {
        n(e).sampleType = me[t];
      },
      __wbg_set_sampler_89cb4a7efcfc6005: function(e, t) {
        n(e).sampler = n(t);
      },
      __wbg_set_shader_location_3fb9f6a012eba494: function(e, t) {
        n(e).shaderLocation = t >>> 0;
      },
      __wbg_set_signal_2dbd377604627f26: function(e, t) {
        n(e).signal = n(t);
      },
      __wbg_set_size_f64_2f591b0654540477: function(e, t) {
        n(e).size = t;
      },
      __wbg_set_size_f64_e844c985b8f95261: function(e, t) {
        n(e).size = t;
      },
      __wbg_set_size_gpu_extent_3d_dict_adf57388ab1d4f18: function(e, t) {
        n(e).size = n(t);
      },
      __wbg_set_src_factor_6f2c9ec8e4d3d979: function(e, t) {
        n(e).srcFactor = $[t];
      },
      __wbg_set_stencil_back_c54d0443b8b6a957: function(e, t) {
        n(e).stencilBack = n(t);
      },
      __wbg_set_stencil_clear_value_a321b0e045bfd8c2: function(e, t) {
        n(e).stencilClearValue = t >>> 0;
      },
      __wbg_set_stencil_front_3ff3f8385852efff: function(e, t) {
        n(e).stencilFront = n(t);
      },
      __wbg_set_stencil_load_op_37d20deccb26a0f1: function(e, t) {
        n(e).stencilLoadOp = z[t];
      },
      __wbg_set_stencil_read_mask_021ef4271b24352c: function(e, t) {
        n(e).stencilReadMask = t >>> 0;
      },
      __wbg_set_stencil_read_only_75fe66a2356d6e92: function(e, t) {
        n(e).stencilReadOnly = t !== 0;
      },
      __wbg_set_stencil_store_op_501f91638dd386e6: function(e, t) {
        n(e).stencilStoreOp = R[t];
      },
      __wbg_set_stencil_write_mask_ec1c12237e094bdd: function(e, t) {
        n(e).stencilWriteMask = t >>> 0;
      },
      __wbg_set_step_mode_3cbbdeba1e5dfd62: function(e, t) {
        n(e).stepMode = ye[t];
      },
      __wbg_set_storage_texture_786aea7c5773b6c1: function(e, t) {
        n(e).storageTexture = n(t);
      },
      __wbg_set_store_op_678f33376d741711: function(e, t) {
        n(e).storeOp = R[t];
      },
      __wbg_set_strip_index_format_70313df755145d5e: function(e, t) {
        n(e).stripIndexFormat = D[t];
      },
      __wbg_set_targets_674b33931e512fb1: function(e, t, _) {
        n(e).targets = x(t, _);
      },
      __wbg_set_texture_95f2bfdf7767e76f: function(e, t) {
        n(e).texture = n(t);
      },
      __wbg_set_texture_a33be3fe02ac6264: function(e, t) {
        n(e).texture = n(t);
      },
      __wbg_set_timestamp_writes_5f240bbaad97fcaf: function(e, t) {
        n(e).timestampWrites = n(t);
      },
      __wbg_set_timestamp_writes_de6a09f299b71b76: function(e, t) {
        n(e).timestampWrites = n(t);
      },
      __wbg_set_tone_mapping_320c1aad31db2e7f: function(e, t) {
        n(e).toneMapping = n(t);
      },
      __wbg_set_topology_b92cfe523bd9653b: function(e, t) {
        n(e).topology = de[t];
      },
      __wbg_set_type_43e0092f16775979: function(e, t) {
        n(e).type = we[t];
      },
      __wbg_set_type_79cec55caf4cdb6d: function(e, t) {
        n(e).type = ie[t];
      },
      __wbg_set_unclipped_depth_32b7caf29fa5633d: function(e, t) {
        n(e).unclippedDepth = t !== 0;
      },
      __wbg_set_usage_1ee33d98267e787d: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_usage_2365e2704b1fdb10: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_usage_d53ee6f0c7aedbfa: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_usage_f3e34822998d2147: function(e, t) {
        n(e).usage = t >>> 0;
      },
      __wbg_set_vertex_77ed7a1229239b5a: function(e, t) {
        n(e).vertex = n(t);
      },
      __wbg_set_view_dimension_893e2d16561e56e8: function(e, t) {
        n(e).viewDimension = j[t];
      },
      __wbg_set_view_dimension_f2c5fe4bf927c3fe: function(e, t) {
        n(e).viewDimension = j[t];
      },
      __wbg_set_view_formats_427069064d8b7139: function(e, t, _) {
        n(e).viewFormats = x(t, _);
      },
      __wbg_set_view_formats_9c2f01a6f3b365c7: function(e, t, _) {
        n(e).viewFormats = x(t, _);
      },
      __wbg_set_view_gpu_texture_view_35f4655788535c4d: function(e, t) {
        n(e).view = n(t);
      },
      __wbg_set_view_gpu_texture_view_a532c825c52042c0: function(e, t) {
        n(e).view = n(t);
      },
      __wbg_set_visibility_d8a6821789538c25: function(e, t) {
        n(e).visibility = t >>> 0;
      },
      __wbg_set_width_0ca908d167690992: function(e, t) {
        n(e).width = t >>> 0;
      },
      __wbg_set_width_b0e1267db4b196b5: function(e, t) {
        n(e).width = t >>> 0;
      },
      __wbg_set_width_b20525f5f4df4eb8: function(e, t) {
        n(e).width = t >>> 0;
      },
      __wbg_set_write_mask_42d89f182ade6b2d: function(e, t) {
        n(e).writeMask = t >>> 0;
      },
      __wbg_set_x_f470b03dd54724cd: function(e, t) {
        n(e).x = t >>> 0;
      },
      __wbg_set_y_4c44eb40ebca5bfc: function(e, t) {
        n(e).y = t >>> 0;
      },
      __wbg_set_z_2e6820ef0f5821ed: function(e, t) {
        n(e).z = t >>> 0;
      },
      __wbg_signal_e5a49a2d9c65b7f6: function(e) {
        const t = n(e).signal;
        return a(t);
      },
      __wbg_stack_3b0d974bbf31e44f: function(e, t) {
        const _ = n(t).stack, r = l(_, o.__wbindgen_export, o.__wbindgen_export2), i = w;
        s().setInt32(e + 4, i, !0), s().setInt32(e + 0, r, !0);
      },
      __wbg_static_accessor_GLOBAL_266715b9d96ba635: function() {
        const e = typeof global > "u" ? null : global;
        return m(e) ? 0 : a(e);
      },
      __wbg_static_accessor_GLOBAL_THIS_10fb7dc1ae063179: function() {
        const e = typeof globalThis > "u" ? null : globalThis;
        return m(e) ? 0 : a(e);
      },
      __wbg_static_accessor_SELF_0b583911f537483a: function() {
        const e = typeof self > "u" ? null : self;
        return m(e) ? 0 : a(e);
      },
      __wbg_static_accessor_WINDOW_d7f903d1508cbdc4: function() {
        const e = typeof window > "u" ? null : window;
        return m(e) ? 0 : a(e);
      },
      __wbg_statusText_bceed558320dd590: function(e, t) {
        const _ = n(t).statusText, r = l(_, o.__wbindgen_export, o.__wbindgen_export2), i = w;
        s().setInt32(e + 4, i, !0), s().setInt32(e + 0, r, !0);
      },
      __wbg_status_ce0a98d3c57125f3: function(e) {
        return n(e).status;
      },
      __wbg_subgroupMaxSize_b43be0aa16182403: function(e) {
        return n(e).subgroupMaxSize;
      },
      __wbg_subgroupMinSize_03feb6ee0cda6775: function(e) {
        return n(e).subgroupMinSize;
      },
      __wbg_submit_077c85cc28e36892: function(e, t, _) {
        n(e).submit(x(t, _));
      },
      __wbg_then_c949d5a25a4e78f8: function(e, t, _) {
        const r = n(e).then(n(t), n(_));
        return a(r);
      },
      __wbg_then_e71170d78fcf8954: function(e, t) {
        const _ = n(e).then(n(t));
        return a(_);
      },
      __wbg_unconfigure_835307f58dc68d80: function(e) {
        n(e).unconfigure();
      },
      __wbg_unmap_6a96b14c9ef5f7f5: function(e) {
        n(e).unmap();
      },
      __wbg_value_f3c585ee8f5ba40c: function(e) {
        const t = n(e).value;
        return a(t);
      },
      __wbg_warn_13abc63e0d4b3527: function(e) {
        console.warn(n(e));
      },
      __wbg_webviewer_new: function(e) {
        const t = W.__wrap(e);
        return a(t);
      },
      __wbg_width_3d0dce3d9892e35e: function(e) {
        return n(e).width;
      },
      __wbg_writeBuffer_f4bb3f54adfe1330: function() {
        return g(function(e, t, _, r, i, u, f) {
          n(e).writeBuffer(n(t), _, B(r, i), u, f);
        }, arguments);
      },
      __wbindgen_generic_0000000000000001: function(e, t) {
        const _ = C(e, t, ne);
        return a(_);
      },
      __wbindgen_generic_0000000000000002: function(e, t) {
        const _ = C(e, t, _e);
        return a(_);
      },
      __wbindgen_generic_0000000000000003: function(e, t) {
        const _ = C(e, t, re);
        return a(_);
      },
      __wbindgen_generic_0000000000000004: function(e, t) {
        const _ = C(e, t, oe);
        return a(_);
      },
      __wbindgen_generic_0000000000000005: function(e) {
        return a(e);
      },
      __wbindgen_generic_0000000000000006: function(e) {
        return a(e);
      },
      __wbindgen_generic_0000000000000007: function(e, t) {
        const _ = B(e, t);
        return a(_);
      },
      __wbindgen_generic_0000000000000008: function(e, t) {
        const _ = d(e, t);
        return a(_);
      },
      __wbindgen_generic_0000000000000009: function(e) {
        const t = BigInt.asUintN(64, e);
        return a(t);
      },
      __wbindgen_object_clone_ref: function(e) {
        const t = n(e);
        return a(t);
      },
      __wbindgen_object_drop_ref: function(e) {
        b(e);
      }
    }
  };
}
function U(c, e, t, _) {
  o.__wasm_bindgen_func_elem_17401(c, e, a(t), a(_));
}
function ne(c, e, t) {
  try {
    const i = o.__wbindgen_add_to_stack_pointer(-16);
    o.__wasm_bindgen_func_elem_17399(i, c, e, a(t));
    var _ = s().getInt32(i + 0, !0), r = s().getInt32(i + 4, !0);
    if (r)
      throw b(_);
  } finally {
    o.__wbindgen_add_to_stack_pointer(16);
  }
}
function _e(c, e, t) {
  try {
    const i = o.__wbindgen_add_to_stack_pointer(-16);
    o.__wasm_bindgen_func_elem_15728(i, c, e, a(t));
    var _ = s().getInt32(i + 0, !0), r = s().getInt32(i + 4, !0);
    if (r)
      throw b(_);
  } finally {
    o.__wbindgen_add_to_stack_pointer(16);
  }
}
function re(c, e, t) {
  try {
    const i = o.__wbindgen_add_to_stack_pointer(-16);
    o.__wasm_bindgen_func_elem_15728_49(i, c, e, a(t));
    var _ = s().getInt32(i + 0, !0), r = s().getInt32(i + 4, !0);
    if (r)
      throw b(_);
  } finally {
    o.__wbindgen_add_to_stack_pointer(16);
  }
}
function oe(c, e, t) {
  try {
    const i = o.__wbindgen_add_to_stack_pointer(-16);
    o.__wasm_bindgen_func_elem_15728_50(i, c, e, a(t));
    var _ = s().getInt32(i + 0, !0), r = s().getInt32(i + 4, !0);
    if (r)
      throw b(_);
  } finally {
    o.__wbindgen_add_to_stack_pointer(16);
  }
}
const G = ["clamp-to-edge", "repeat", "mirror-repeat"], E = ["auto"], $ = ["zero", "one", "src", "one-minus-src", "src-alpha", "one-minus-src-alpha", "dst", "one-minus-dst", "dst-alpha", "one-minus-dst-alpha", "src-alpha-saturated", "constant", "one-minus-constant", "src1", "one-minus-src1", "src1-alpha", "one-minus-src1-alpha"], ce = ["add", "subtract", "reverse-subtract", "min", "max"], ie = ["uniform", "storage", "read-only-storage"], ae = ["opaque", "premultiplied"], se = ["standard", "extended"], F = ["never", "less", "equal", "less-equal", "greater", "not-equal", "greater-equal", "always"], ue = ["none", "front", "back"], N = ["nearest", "linear"], fe = ["ccw", "cw"], D = ["uint16", "uint32"], z = ["load", "clear"], be = ["nearest", "linear"], ge = ["low-power", "high-performance"], de = ["point-list", "line-list", "line-strip", "triangle-list", "triangle-strip"], we = ["filtering", "non-filtering", "comparison"], L = ["keep", "zero", "replace", "invert", "increment-clamp", "decrement-clamp", "increment-wrap", "decrement-wrap"], le = ["write-only", "read-only", "read-write"], R = ["store", "discard"], H = ["all", "stencil-only", "depth-only"], pe = ["1d", "2d", "3d"], h = ["r8unorm", "r8snorm", "r8uint", "r8sint", "r16unorm", "r16snorm", "r16uint", "r16sint", "r16float", "rg8unorm", "rg8snorm", "rg8uint", "rg8sint", "r32uint", "r32sint", "r32float", "rg16unorm", "rg16snorm", "rg16uint", "rg16sint", "rg16float", "rgba8unorm", "rgba8unorm-srgb", "rgba8snorm", "rgba8uint", "rgba8sint", "bgra8unorm", "bgra8unorm-srgb", "rgb9e5ufloat", "rgb10a2uint", "rgb10a2unorm", "rg11b10ufloat", "rg32uint", "rg32sint", "rg32float", "rgba16unorm", "rgba16snorm", "rgba16uint", "rgba16sint", "rgba16float", "rgba32uint", "rgba32sint", "rgba32float", "stencil8", "depth16unorm", "depth24plus", "depth24plus-stencil8", "depth32float", "depth32float-stencil8", "bc1-rgba-unorm", "bc1-rgba-unorm-srgb", "bc2-rgba-unorm", "bc2-rgba-unorm-srgb", "bc3-rgba-unorm", "bc3-rgba-unorm-srgb", "bc4-r-unorm", "bc4-r-snorm", "bc5-rg-unorm", "bc5-rg-snorm", "bc6h-rgb-ufloat", "bc6h-rgb-float", "bc7-rgba-unorm", "bc7-rgba-unorm-srgb", "etc2-rgb8unorm", "etc2-rgb8unorm-srgb", "etc2-rgb8a1unorm", "etc2-rgb8a1unorm-srgb", "etc2-rgba8unorm", "etc2-rgba8unorm-srgb", "eac-r11unorm", "eac-r11snorm", "eac-rg11unorm", "eac-rg11snorm", "astc-4x4-unorm", "astc-4x4-unorm-srgb", "astc-5x4-unorm", "astc-5x4-unorm-srgb", "astc-5x5-unorm", "astc-5x5-unorm-srgb", "astc-6x5-unorm", "astc-6x5-unorm-srgb", "astc-6x6-unorm", "astc-6x6-unorm-srgb", "astc-8x5-unorm", "astc-8x5-unorm-srgb", "astc-8x6-unorm", "astc-8x6-unorm-srgb", "astc-8x8-unorm", "astc-8x8-unorm-srgb", "astc-10x5-unorm", "astc-10x5-unorm-srgb", "astc-10x6-unorm", "astc-10x6-unorm-srgb", "astc-10x8-unorm", "astc-10x8-unorm-srgb", "astc-10x10-unorm", "astc-10x10-unorm-srgb", "astc-12x10-unorm", "astc-12x10-unorm-srgb", "astc-12x12-unorm", "astc-12x12-unorm-srgb"], me = ["float", "unfilterable-float", "depth", "sint", "uint"], j = ["1d", "2d", "2d-array", "cube", "cube-array", "3d"], xe = ["uint8", "uint8x2", "uint8x4", "sint8", "sint8x2", "sint8x4", "unorm8", "unorm8x2", "unorm8x4", "snorm8", "snorm8x2", "snorm8x4", "uint16", "uint16x2", "uint16x4", "sint16", "sint16x2", "sint16x4", "unorm16", "unorm16x2", "unorm16x4", "snorm16", "snorm16x2", "snorm16x4", "float16", "float16x2", "float16x4", "float32", "float32x2", "float32x3", "float32x4", "uint32", "uint32x2", "uint32x3", "uint32x4", "sint32", "sint32x2", "sint32x3", "sint32x4", "unorm10-10-10-2", "unorm8x4-bgra"], ye = ["vertex", "instance"], X = typeof FinalizationRegistry > "u" ? { register: () => {
}, unregister: () => {
} } : new FinalizationRegistry((c) => o.__wbg_webviewer_free(c, 1));
function a(c) {
  T === y.length && y.push(y.length + 1);
  const e = T;
  return T = y[e], y[e] = c, e;
}
const Y = typeof FinalizationRegistry > "u" ? { register: () => {
}, unregister: () => {
} } : new FinalizationRegistry((c) => o.__wbindgen_export5(c.a, c.b));
function q(c) {
  const e = typeof c;
  if (e == "number" || e == "boolean" || c == null)
    return `${c}`;
  if (e == "string")
    return `"${c}"`;
  if (e == "symbol") {
    const r = c.description;
    return r == null ? "Symbol" : `Symbol(${r})`;
  }
  if (e == "function") {
    const r = c.name;
    return typeof r == "string" && r.length > 0 ? `Function(${r})` : "Function";
  }
  if (Array.isArray(c)) {
    const r = c.length;
    let i = "[";
    r > 0 && (i += q(c[0]));
    for (let u = 1; u < r; u++)
      i += ", " + q(c[u]);
    return i += "]", i;
  }
  const t = /\[object ([^\]]+)\]/.exec(toString.call(c));
  let _;
  if (t && t.length > 1)
    _ = t[1];
  else
    return toString.call(c);
  if (_ == "Object")
    try {
      return "Object(" + JSON.stringify(c) + ")";
    } catch {
      return "Object";
    }
  return c instanceof Error ? `${c.name}: ${c.message}
${c.stack}` : _;
}
function he(c) {
  c < 1028 || (y[c] = T, T = c);
}
function x(c, e) {
  c = c >>> 0;
  const t = s(), _ = [];
  for (let r = c; r < c + 4 * e; r += 4)
    _.push(n(t.getUint32(r, !0)));
  return _;
}
function J(c, e) {
  return c = c >>> 0, ve().subarray(c / 4, c / 4 + e);
}
function B(c, e) {
  return c = c >>> 0, S().subarray(c / 1, c / 1 + e);
}
let v = null;
function s() {
  return (v === null || v.buffer.detached === !0 || v.buffer.detached === void 0 && v.buffer !== o.memory.buffer) && (v = new DataView(o.memory.buffer)), v;
}
function d(c, e) {
  return Be(c >>> 0, e);
}
let I = null;
function ve() {
  return (I === null || I.byteLength === 0) && (I = new Uint32Array(o.memory.buffer)), I;
}
let A = null;
function S() {
  return (A === null || A.byteLength === 0) && (A = new Uint8Array(o.memory.buffer)), A;
}
function n(c) {
  return y[c];
}
function g(c, e) {
  try {
    return c.apply(this, e);
  } catch (t) {
    o.__wbindgen_export3(a(t));
  }
}
let y = new Array(1024).fill(void 0);
y.push(void 0, null, !0, !1);
let T = y.length;
function m(c) {
  return c == null;
}
function C(c, e, t) {
  const _ = { a: c, b: e, cnt: 1 }, r = (...i) => {
    _.cnt++;
    const u = _.a;
    _.a = 0;
    try {
      return t(u, _.b, ...i);
    } finally {
      _.a = u, r._wbg_cb_unref();
    }
  };
  return r._wbg_cb_unref = () => {
    --_.cnt === 0 && (o.__wbindgen_export5(_.a, _.b), _.a = 0, Y.unregister(_));
  }, Y.register(r, _, _), r;
}
function Se(c, e) {
  const t = e(c.length * 1, 1) >>> 0;
  return S().set(c, t / 1), w = c.length, t;
}
function l(c, e, t) {
  if (t === void 0) {
    const f = P.encode(c), p = e(f.length, 1) >>> 0;
    return S().subarray(p, p + f.length).set(f), w = f.length, p;
  }
  let _ = c.length, r = e(_, 1) >>> 0;
  const i = S();
  let u = 0;
  for (; u < _; u++) {
    const f = c.charCodeAt(u);
    if (f > 127) break;
    i[r + u] = f;
  }
  if (u !== _) {
    u !== 0 && (c = c.slice(u)), r = t(r, _, _ = u + c.length * 3, 1) >>> 0;
    const f = S().subarray(r + u, r + _), p = P.encodeInto(c, f);
    u += p.written, r = t(r, _, u, 1) >>> 0;
  }
  return w = u, r;
}
function b(c) {
  const e = n(c);
  return he(c), e;
}
let M = new TextDecoder("utf-8", { ignoreBOM: !0, fatal: !0 });
M.decode();
const ke = 2146435072;
let V = 0;
function Be(c, e) {
  return V += e, V >= ke && (M = new TextDecoder("utf-8", { ignoreBOM: !0, fatal: !0 }), M.decode(), V = e), M.decode(S().subarray(c, c + e));
}
const P = new TextEncoder();
"encodeInto" in P || (P.encodeInto = function(c, e) {
  const t = P.encode(c);
  return e.set(t), {
    read: c.length,
    written: t.length
  };
});
let w = 0, o;
function K(c, e) {
  return o = c.exports, v = null, I = null, A = null, o.__wbindgen_start(), o;
}
async function Ie(c, e) {
  if (typeof Response == "function" && c instanceof Response) {
    if (!c.ok)
      throw new Error(`failed to fetch Wasm: ${c.status} ${c.statusText} fetching '${c.url}'`);
    if (typeof WebAssembly.instantiateStreaming == "function")
      try {
        return await WebAssembly.instantiateStreaming(c, e);
      } catch (r) {
        if (t(c.type) && c.headers.get("Content-Type") !== "application/wasm")
          console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", r);
        else
          throw r;
      }
    const _ = await c.arrayBuffer();
    return await WebAssembly.instantiate(_, e);
  } else {
    const _ = await WebAssembly.instantiate(c, e);
    return _ instanceof WebAssembly.Instance ? { instance: _, module: c } : _;
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
function Te(c) {
  if (o !== void 0) return o;
  c !== void 0 && (Object.getPrototypeOf(c) === Object.prototype ? { module: c } = c : console.warn("using deprecated parameters for `initSync()`; pass a single object instead"));
  const e = Z();
  c instanceof WebAssembly.Module || (c = new WebAssembly.Module(c));
  const t = new WebAssembly.Instance(c, e);
  return K(t);
}
async function Pe(c) {
  if (o !== void 0) return o;
  c !== void 0 && (Object.getPrototypeOf(c) === Object.prototype ? { module_or_path: c } = c : console.warn("using deprecated parameters for the initialization function; pass a single object instead")), c === void 0 && (c = new URL("patinae_web_bg.wasm", /* @__PURE__ */ ((r) => r)(import.meta.url)));
  const e = Z();
  (typeof c == "string" || typeof Request == "function" && c instanceof Request || typeof URL == "function" && c instanceof URL) && (c = fetch(c));
  const { instance: t, module: _ } = await Ie(await c, e);
  return K(t);
}
export {
  W as WebViewer,
  Pe as default,
  Ae as init,
  Te as initSync
};
