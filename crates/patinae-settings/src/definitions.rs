//! Setting definitions with stable numeric IDs.
//!
//! This module contains the complete list of settings.
//! Setting indices are stable for PSE session format compatibility.

use crate::setting::{Setting, SettingLevel, SettingType, SettingValue};

/// Total number of settings
pub const SETTING_COUNT: usize = 813;

// =============================================================================
// Setting ID Constants
// =============================================================================

/// Setting indices - must remain stable for session compatibility
#[allow(non_upper_case_globals)]
pub mod id {
    // 0-49
    pub const bonding_vdw_cutoff: u16 = 0;
    pub const min_mesh_spacing: u16 = 1;
    pub const dot_density: u16 = 2;
    pub const dot_mode: u16 = 3;
    pub const solvent_radius: u16 = 4;
    pub const sel_counter: u16 = 5;
    pub const bg_rgb: u16 = 6;
    pub const ambient: u16 = 7;
    pub const direct: u16 = 8;
    pub const reflect: u16 = 9;
    pub const light: u16 = 10;
    pub const power: u16 = 11;
    pub const antialias: u16 = 12;
    pub const cavity_cull: u16 = 13;
    pub const gl_ambient: u16 = 14;
    pub const single_image: u16 = 15;
    pub const movie_delay: u16 = 16;
    pub const ribbon_power: u16 = 17;
    pub const ribbon_power_b: u16 = 18;
    pub const ribbon_sampling: u16 = 19;
    pub const ribbon_radius: u16 = 20;
    pub const stick_radius: u16 = 21;
    pub const hash_max: u16 = 22;
    pub const orthoscopic: u16 = 23;
    pub const spec_reflect: u16 = 24;
    pub const spec_power: u16 = 25;
    pub const sweep_angle: u16 = 26;
    pub const sweep_speed: u16 = 27;
    pub const dot_hydrogens: u16 = 28;
    pub const dot_radius: u16 = 29;
    pub const ray_trace_frames: u16 = 30;
    pub const cache_frames: u16 = 31;
    pub const trim_dots: u16 = 32;
    pub const cull_spheres: u16 = 33;
    pub const test1: u16 = 34;
    pub const test2: u16 = 35;
    pub const surface_best: u16 = 36;
    pub const surface_normal: u16 = 37;
    pub const surface_quality: u16 = 38;
    pub const surface_proximity: u16 = 39;
    pub const normal_workaround: u16 = 40;
    pub const stereo_angle: u16 = 41;
    pub const stereo_shift: u16 = 42;
    pub const line_smooth: u16 = 43;
    pub const line_width: u16 = 44;
    pub const half_bonds: u16 = 45;
    pub const stick_quality: u16 = 46;
    pub const stick_overlap: u16 = 47;
    pub const stick_nub: u16 = 48;
    pub const all_states: u16 = 49;

    // 50-99
    pub const pickable: u16 = 50;
    pub const auto_show_lines: u16 = 51;
    pub const idle_delay: u16 = 52;
    pub const no_idle: u16 = 53;
    pub const fast_idle: u16 = 54;
    pub const slow_idle: u16 = 55;
    pub const rock_delay: u16 = 56;
    pub const dist_counter: u16 = 57;
    pub const dash_length: u16 = 58;
    pub const dash_gap: u16 = 59;
    pub const auto_zoom: u16 = 60;
    pub const overlay: u16 = 61;
    pub const text: u16 = 62;
    pub const button_mode: u16 = 63;
    pub const valence: u16 = 64;
    pub const nonbonded_size: u16 = 65;
    pub const label_color: u16 = 66;
    pub const ray_trace_fog: u16 = 67;
    pub const spheroid_scale: u16 = 68;
    pub const ray_trace_fog_start: u16 = 69;
    pub const spheroid_smooth: u16 = 70;
    pub const spheroid_fill: u16 = 71;
    pub const auto_show_nonbonded: u16 = 72;
    pub const cache_display: u16 = 73;
    pub const mesh_radius: u16 = 74;
    pub const backface_cull: u16 = 75;
    pub const gamma: u16 = 76;
    pub const dot_width: u16 = 77;
    pub const auto_show_selections: u16 = 78;
    pub const auto_hide_selections: u16 = 79;
    pub const selection_width: u16 = 80;
    pub const selection_overlay: u16 = 81;
    pub const static_singletons: u16 = 82;
    pub const unused_83: u16 = 83;
    pub const depth_cue: u16 = 84;
    pub const specular: u16 = 85;
    pub const shininess: u16 = 86;
    pub const sphere_quality: u16 = 87;
    pub const fog: u16 = 88;
    pub const isomesh_auto_state: u16 = 89;
    pub const mesh_width: u16 = 90;
    pub const cartoon_sampling: u16 = 91;
    pub const cartoon_loop_radius: u16 = 92;
    pub const cartoon_loop_quality: u16 = 93;
    pub const cartoon_power: u16 = 94;
    pub const cartoon_power_b: u16 = 95;
    pub const cartoon_rect_length: u16 = 96;
    pub const cartoon_rect_width: u16 = 97;
    pub const internal_gui_width: u16 = 98;
    pub const internal_gui: u16 = 99;

    // 100-149
    pub const cartoon_oval_length: u16 = 100;
    pub const cartoon_oval_width: u16 = 101;
    pub const cartoon_oval_quality: u16 = 102;
    pub const cartoon_tube_radius: u16 = 103;
    pub const cartoon_tube_quality: u16 = 104;
    pub const cartoon_debug: u16 = 105;
    pub const ribbon_width: u16 = 106;
    pub const dash_width: u16 = 107;
    pub const dash_radius: u16 = 108;
    pub const cgo_ray_width_scale: u16 = 109;
    pub const line_radius: u16 = 110;
    pub const cartoon_round_helices: u16 = 111;
    pub const cartoon_refine_normals: u16 = 112;
    pub const cartoon_flat_sheets: u16 = 113;
    pub const cartoon_smooth_loops: u16 = 114;
    pub const cartoon_dumbbell_length: u16 = 115;
    pub const cartoon_dumbbell_width: u16 = 116;
    pub const cartoon_dumbbell_radius: u16 = 117;
    pub const cartoon_fancy_helices: u16 = 118;
    pub const cartoon_fancy_sheets: u16 = 119;
    pub const ignore_pdb_segi: u16 = 120;
    pub const ribbon_throw: u16 = 121;
    pub const cartoon_throw: u16 = 122;
    pub const cartoon_refine: u16 = 123;
    pub const cartoon_refine_tips: u16 = 124;
    pub const cartoon_discrete_colors: u16 = 125;
    pub const normalize_ccp4_maps: u16 = 126;
    pub const surface_poor: u16 = 127;
    pub const internal_feedback: u16 = 128;
    pub const cgo_line_width: u16 = 129;
    pub const cgo_line_radius: u16 = 130;
    pub const logging: u16 = 131;
    pub const robust_logs: u16 = 132;
    pub const log_box_selections: u16 = 133;
    pub const log_conformations: u16 = 134;
    pub const valence_size: u16 = 135;
    pub const surface_miserable: u16 = 136;
    pub const ray_opaque_background: u16 = 137;
    pub const surface_transparency: u16 = 138;
    pub const ray_texture: u16 = 139;
    pub const ray_texture_settings: u16 = 140;
    pub const suspend_updates: u16 = 141;
    pub const full_screen: u16 = 142;
    pub const surface_mode: u16 = 143;
    pub const surface_color: u16 = 144;
    pub const mesh_mode: u16 = 145;
    pub const mesh_color: u16 = 146;
    pub const auto_indicate_flags: u16 = 147;
    pub const surface_debug: u16 = 148;
    pub const ray_improve_shadows: u16 = 149;

    // 150-199
    pub const smooth_color_triangle: u16 = 150;
    pub const ray_default_renderer: u16 = 151;
    pub const field_of_view: u16 = 152;
    pub const reflect_power: u16 = 153;
    pub const preserve_chempy_ids: u16 = 154;
    pub const sphere_scale: u16 = 155;
    pub const two_sided_lighting: u16 = 156;
    pub const secondary_structure: u16 = 157;
    pub const auto_remove_hydrogens: u16 = 158;
    pub const raise_exceptions: u16 = 159;
    pub const stop_on_exceptions: u16 = 160;
    pub const sculpting: u16 = 161;
    pub const auto_sculpt: u16 = 162;
    pub const sculpt_vdw_scale: u16 = 163;
    pub const sculpt_vdw_scale14: u16 = 164;
    pub const sculpt_vdw_weight: u16 = 165;
    pub const sculpt_vdw_weight14: u16 = 166;
    pub const sculpt_bond_weight: u16 = 167;
    pub const sculpt_angl_weight: u16 = 168;
    pub const sculpt_pyra_weight: u16 = 169;
    pub const sculpt_plan_weight: u16 = 170;
    pub const sculpting_cycles: u16 = 171;
    pub const sphere_transparency: u16 = 172;
    pub const sphere_color: u16 = 173;
    pub const sculpt_field_mask: u16 = 174;
    pub const sculpt_hb_overlap: u16 = 175;
    pub const sculpt_hb_overlap_base: u16 = 176;
    pub const legacy_vdw_radii: u16 = 177;
    pub const sculpt_memory: u16 = 178;
    pub const connect_mode: u16 = 179;
    pub const cartoon_cylindrical_helices: u16 = 180;
    pub const cartoon_helix_radius: u16 = 181;
    pub const connect_cutoff: u16 = 182;
    pub const save_pdb_ss: u16 = 183;
    pub const sculpt_line_weight: u16 = 184;
    pub const fit_iterations: u16 = 185;
    pub const fit_tolerance: u16 = 186;
    pub const batch_prefix: u16 = 187;
    pub const stereo_mode: u16 = 188;
    pub const cgo_sphere_quality: u16 = 189;
    pub const pdb_literal_names: u16 = 190;
    pub const wrap_output: u16 = 191;
    pub const fog_start: u16 = 192;
    pub const state: u16 = 193;
    pub const frame: u16 = 194;
    pub const ray_shadow: u16 = 195;
    pub const ribbon_trace_atoms: u16 = 196;
    pub const security: u16 = 197;
    pub const stick_transparency: u16 = 198;
    pub const ray_transparency_shadows: u16 = 199;

    // 200-249
    pub const session_version_check: u16 = 200;
    pub const ray_transparency_specular: u16 = 201;
    pub const stereo_double_pump_mono: u16 = 202;
    pub const sphere_solvent: u16 = 203;
    pub const mesh_quality: u16 = 204;
    pub const mesh_solvent: u16 = 205;
    pub const dot_solvent: u16 = 206;
    pub const ray_shadow_fudge: u16 = 207;
    pub const ray_triangle_fudge: u16 = 208;
    pub const debug_pick: u16 = 209;
    pub const dot_color: u16 = 210;
    pub const mouse_limit: u16 = 211;
    pub const mouse_scale: u16 = 212;
    pub const transparency_mode: u16 = 213;
    pub const clamp_colors: u16 = 214;
    pub const pymol_space_max_red: u16 = 215;
    pub const pymol_space_max_green: u16 = 216;
    pub const pymol_space_max_blue: u16 = 217;
    pub const pymol_space_min_factor: u16 = 218;
    pub const roving_origin: u16 = 219;
    pub const roving_lines: u16 = 220;
    pub const roving_sticks: u16 = 221;
    pub const roving_spheres: u16 = 222;
    pub const roving_labels: u16 = 223;
    pub const roving_delay: u16 = 224;
    pub const roving_selection: u16 = 225;
    pub const roving_byres: u16 = 226;
    pub const roving_ribbon: u16 = 227;
    pub const roving_cartoon: u16 = 228;
    pub const roving_polar_contacts: u16 = 229;
    pub const roving_polar_cutoff: u16 = 230;
    pub const roving_nonbonded: u16 = 231;
    pub const float_labels: u16 = 232;
    pub const roving_detail: u16 = 233;
    pub const roving_nb_spheres: u16 = 234;
    pub const ribbon_color: u16 = 235;
    pub const cartoon_color: u16 = 236;
    pub const ribbon_smooth: u16 = 237;
    pub const auto_color: u16 = 238;
    pub const auto_color_next: u16 = 239;
    pub const ray_interior_color: u16 = 240;
    pub const cartoon_highlight_color: u16 = 241;
    pub const coulomb_units_factor: u16 = 242;
    pub const coulomb_dielectric: u16 = 243;
    pub const ray_interior_shadows: u16 = 244;
    pub const ray_interior_texture: u16 = 245;
    pub const roving_map1_name: u16 = 246;
    pub const roving_map2_name: u16 = 247;
    pub const roving_map3_name: u16 = 248;
    pub const roving_map1_level: u16 = 249;

    // 250-299
    pub const roving_map2_level: u16 = 250;
    pub const roving_map3_level: u16 = 251;
    pub const roving_isomesh: u16 = 252;
    pub const roving_isosurface: u16 = 253;
    pub const scenes_changed: u16 = 254;
    pub const gaussian_b_adjust: u16 = 255;
    pub const pdb_standard_order: u16 = 256;
    pub const cartoon_smooth_first: u16 = 257;
    pub const cartoon_smooth_last: u16 = 258;
    pub const cartoon_smooth_cycles: u16 = 259;
    pub const cartoon_flat_cycles: u16 = 260;
    pub const max_threads: u16 = 261;
    pub const show_progress: u16 = 262;
    pub const use_display_lists: u16 = 263;
    pub const cache_memory: u16 = 264;
    pub const simplify_display_lists: u16 = 265;
    pub const retain_order: u16 = 266;
    pub const pdb_hetatm_sort: u16 = 267;
    pub const pdb_use_ter_records: u16 = 268;
    pub const cartoon_trace_atoms: u16 = 269;
    pub const ray_oversample_cutoff: u16 = 270;
    pub const gaussian_resolution: u16 = 271;
    pub const gaussian_b_floor: u16 = 272;
    pub const sculpt_nb_interval: u16 = 273;
    pub const sculpt_tors_weight: u16 = 274;
    pub const sculpt_tors_tolerance: u16 = 275;
    pub const stick_ball: u16 = 276;
    pub const stick_ball_ratio: u16 = 277;
    pub const stick_fixed_radius: u16 = 278;
    pub const cartoon_transparency: u16 = 279;
    pub const dash_round_ends: u16 = 280;
    pub const h_bond_max_angle: u16 = 281;
    pub const h_bond_cutoff_center: u16 = 282;
    pub const h_bond_cutoff_edge: u16 = 283;
    pub const h_bond_power_a: u16 = 284;
    pub const h_bond_power_b: u16 = 285;
    pub const h_bond_cone: u16 = 286;
    pub const ss_helix_psi_target: u16 = 287;
    pub const ss_helix_psi_include: u16 = 288;
    pub const ss_helix_psi_exclude: u16 = 289;
    pub const ss_helix_phi_target: u16 = 290;
    pub const ss_helix_phi_include: u16 = 291;
    pub const ss_helix_phi_exclude: u16 = 292;
    pub const ss_strand_psi_target: u16 = 293;
    pub const ss_strand_psi_include: u16 = 294;
    pub const ss_strand_psi_exclude: u16 = 295;
    pub const ss_strand_phi_target: u16 = 296;
    pub const ss_strand_phi_include: u16 = 297;
    pub const ss_strand_phi_exclude: u16 = 298;
    pub const movie_loop: u16 = 299;

    // 300-349
    pub const pdb_retain_ids: u16 = 300;
    pub const pdb_no_end_record: u16 = 301;
    pub const cgo_dot_width: u16 = 302;
    pub const cgo_dot_radius: u16 = 303;
    pub const defer_updates: u16 = 304;
    pub const normalize_o_maps: u16 = 305;
    pub const swap_dsn6_bytes: u16 = 306;
    pub const pdb_insertions_go_first: u16 = 307;
    pub const roving_origin_z: u16 = 308;
    pub const roving_origin_z_cushion: u16 = 309;
    pub const specular_intensity: u16 = 310;
    pub const overlay_lines: u16 = 311;
    pub const ray_transparency_spec_cut: u16 = 312;
    pub const internal_prompt: u16 = 313;
    pub const normalize_grd_maps: u16 = 314;
    pub const ray_blend_colors: u16 = 315;
    pub const ray_blend_red: u16 = 316;
    pub const ray_blend_green: u16 = 317;
    pub const ray_blend_blue: u16 = 318;
    pub const png_screen_gamma: u16 = 319;
    pub const png_file_gamma: u16 = 320;
    pub const editor_label_fragments: u16 = 321;
    pub const internal_gui_control_size: u16 = 322;
    pub const auto_dss: u16 = 323;
    pub const transparency_picking_mode: u16 = 324;
    pub const virtual_trackball: u16 = 325;
    pub const pdb_reformat_names_mode: u16 = 326;
    pub const ray_pixel_scale: u16 = 327;
    pub const label_font_id: u16 = 328;
    pub const pdb_conect_all: u16 = 329;
    pub const button_mode_name: u16 = 330;
    pub const surface_type: u16 = 331;
    pub const dot_normals: u16 = 332;
    pub const session_migration: u16 = 333;
    pub const mesh_normals: u16 = 334;
    pub const mesh_type: u16 = 335;
    pub const dot_lighting: u16 = 336;
    pub const mesh_lighting: u16 = 337;
    pub const surface_solvent: u16 = 338;
    pub const triangle_max_passes: u16 = 339;
    pub const ray_interior_reflect: u16 = 340;
    pub const internal_gui_mode: u16 = 341;
    pub const surface_carve_selection: u16 = 342;
    pub const surface_carve_state: u16 = 343;
    pub const surface_carve_cutoff: u16 = 344;
    pub const surface_clear_selection: u16 = 345;
    pub const surface_clear_state: u16 = 346;
    pub const surface_clear_cutoff: u16 = 347;
    pub const surface_trim_cutoff: u16 = 348;
    pub const surface_trim_factor: u16 = 349;

    // 350-399
    pub const ray_max_passes: u16 = 350;
    pub const active_selections: u16 = 351;
    pub const ray_transparency_contrast: u16 = 352;
    pub const seq_view: u16 = 353;
    pub const mouse_selection_mode: u16 = 354;
    pub const seq_view_label_spacing: u16 = 355;
    pub const seq_view_label_start: u16 = 356;
    pub const seq_view_format: u16 = 357;
    pub const seq_view_location: u16 = 358;
    pub const seq_view_overlay: u16 = 359;
    pub const auto_classify_atoms: u16 = 360;
    pub const cartoon_nucleic_acid_mode: u16 = 361;
    pub const seq_view_color: u16 = 362;
    pub const seq_view_label_mode: u16 = 363;
    pub const surface_ramp_above_mode: u16 = 364;
    pub const stereo: u16 = 365;
    pub const wizard_prompt_mode: u16 = 366;
    pub const coulomb_cutoff: u16 = 367;
    pub const slice_track_camera: u16 = 368;
    pub const slice_height_scale: u16 = 369;
    pub const slice_height_map: u16 = 370;
    pub const slice_grid: u16 = 371;
    pub const slice_dynamic_grid: u16 = 372;
    pub const slice_dynamic_grid_resolution: u16 = 373;
    pub const pdb_insure_orthogonal: u16 = 374;
    pub const ray_direct_shade: u16 = 375;
    pub const stick_color: u16 = 376;
    pub const cartoon_putty_radius: u16 = 377;
    pub const cartoon_putty_quality: u16 = 378;
    pub const cartoon_putty_scale_min: u16 = 379;
    pub const cartoon_putty_scale_max: u16 = 380;
    pub const cartoon_putty_scale_power: u16 = 381;
    pub const cartoon_putty_range: u16 = 382;
    pub const cartoon_side_chain_helper: u16 = 383;
    pub const surface_optimize_subsets: u16 = 384;
    pub const multiplex: u16 = 385;
    pub const texture_fonts: u16 = 386;
    pub const pqr_no_chain_id: u16 = 387;
    pub const animation: u16 = 388;
    pub const animation_duration: u16 = 389;
    pub const scene_animation: u16 = 390;
    pub const line_stick_helper: u16 = 391;
    pub const ray_orthoscopic: u16 = 392;
    pub const ribbon_side_chain_helper: u16 = 393;
    pub const selection_width_max: u16 = 394;
    pub const selection_width_scale: u16 = 395;
    pub const scene_current_name: u16 = 396;
    pub const presentation: u16 = 397;
    pub const presentation_mode: u16 = 398;
    pub const pdb_truncate_residue_name: u16 = 399;

    // 400-449
    pub const scene_loop: u16 = 400;
    pub const sweep_mode: u16 = 401;
    pub const sweep_phase: u16 = 402;
    pub const scene_restart_movie_delay: u16 = 403;
    pub const mouse_restart_movie_delay: u16 = 404;
    pub const angle_size: u16 = 405;
    pub const angle_label_position: u16 = 406;
    pub const dihedral_size: u16 = 407;
    pub const dihedral_label_position: u16 = 408;
    pub const defer_builds_mode: u16 = 409;
    pub const seq_view_discrete_by_state: u16 = 410;
    pub const scene_animation_duration: u16 = 411;
    pub const wildcard: u16 = 412;
    pub const atom_name_wildcard: u16 = 413;
    pub const ignore_case: u16 = 414;
    pub const presentation_auto_quit: u16 = 415;
    pub const editor_auto_dihedral: u16 = 416;
    pub const presentation_auto_start: u16 = 417;
    pub const validate_object_names: u16 = 418;
    pub const unused_boolean_def_true: u16 = 419;
    pub const auto_show_spheres: u16 = 420;
    pub const sphere_mode: u16 = 421;
    pub const sphere_point_max_size: u16 = 422;
    pub const sphere_point_size: u16 = 423;
    pub const pdb_honor_model_number: u16 = 424;
    pub const rank_assisted_sorts: u16 = 425;
    pub const ribbon_nucleic_acid_mode: u16 = 426;
    pub const cartoon_ring_mode: u16 = 427;
    pub const cartoon_ring_width: u16 = 428;
    pub const cartoon_ring_color: u16 = 429;
    pub const cartoon_ring_finder: u16 = 430;
    pub const cartoon_tube_cap: u16 = 431;
    pub const cartoon_loop_cap: u16 = 432;
    pub const nvidia_bugs: u16 = 433;
    pub const image_dots_per_inch: u16 = 434;
    pub const opaque_background: u16 = 435;
    pub const draw_frames: u16 = 436;
    pub const show_alpha_checker: u16 = 437;
    pub const matrix_mode: u16 = 438;
    pub const editor_auto_origin: u16 = 439;
    pub const session_file: u16 = 440;
    pub const cgo_transparency: u16 = 441;
    pub const legacy_mouse_zoom: u16 = 442;
    pub const auto_number_selections: u16 = 443;
    pub const sculpt_vdw_vis_mode: u16 = 444;
    pub const sculpt_vdw_vis_min: u16 = 445;
    pub const sculpt_vdw_vis_mid: u16 = 446;
    pub const sculpt_vdw_vis_max: u16 = 447;
    pub const cartoon_ladder_mode: u16 = 448;
    pub const cartoon_ladder_radius: u16 = 449;

    // 450-499
    pub const cartoon_ladder_color: u16 = 450;
    pub const cartoon_nucleic_acid_color: u16 = 451;
    pub const cartoon_ring_transparency: u16 = 452;
    pub const label_size: u16 = 453;
    pub const spec_direct: u16 = 454;
    pub const light_count: u16 = 455;
    pub const light2: u16 = 456;
    pub const light3: u16 = 457;
    pub const hide_underscore_names: u16 = 458;
    pub const selection_round_points: u16 = 459;
    pub const distance_exclusion: u16 = 460;
    pub const h_bond_exclusion: u16 = 461;
    pub const label_shadow_mode: u16 = 462;
    pub const light4: u16 = 463;
    pub const light5: u16 = 464;
    pub const light6: u16 = 465;
    pub const light7: u16 = 466;
    pub const label_outline_color: u16 = 467;
    pub const ray_trace_mode: u16 = 468;
    pub const ray_trace_gain: u16 = 469;
    pub const selection_visible_only: u16 = 470;
    pub const label_position: u16 = 471;
    pub const ray_trace_depth_factor: u16 = 472;
    pub const ray_trace_slope_factor: u16 = 473;
    pub const ray_trace_disco_factor: u16 = 474;
    pub const ray_shadow_decay_factor: u16 = 475;
    pub const ray_interior_mode: u16 = 476;
    pub const ray_legacy_lighting: u16 = 477;
    pub const sculpt_auto_center: u16 = 478;
    pub const pdb_discrete_chains: u16 = 479;
    pub const pdb_unbond_cations: u16 = 480;
    pub const sculpt_tri_scale: u16 = 481;
    pub const sculpt_tri_weight: u16 = 482;
    pub const sculpt_tri_min: u16 = 483;
    pub const sculpt_tri_max: u16 = 484;
    pub const sculpt_tri_mode: u16 = 485;
    pub const pdb_echo_tags: u16 = 486;
    pub const connect_bonded: u16 = 487;
    pub const spec_direct_power: u16 = 488;
    pub const light8: u16 = 489;
    pub const light9: u16 = 490;
    pub const ray_shadow_decay_range: u16 = 491;
    pub const spec_count: u16 = 492;
    pub const sculpt_min_scale: u16 = 493;
    pub const sculpt_min_weight: u16 = 494;
    pub const sculpt_min_min: u16 = 495;
    pub const sculpt_min_max: u16 = 496;
    pub const sculpt_max_scale: u16 = 497;
    pub const sculpt_max_weight: u16 = 498;
    pub const sculpt_max_min: u16 = 499;

    // 500-549
    pub const sculpt_max_max: u16 = 500;
    pub const surface_circumscribe: u16 = 501;
    pub const sculpt_avd_weight: u16 = 502;
    pub const sculpt_avd_gap: u16 = 503;
    pub const sculpt_avd_range: u16 = 504;
    pub const sculpt_avd_excl: u16 = 505;
    pub const async_builds: u16 = 506;
    pub const fetch_path: u16 = 507;
    pub const cartoon_ring_radius: u16 = 508;
    pub const ray_color_ramps: u16 = 509;
    pub const ray_hint_camera: u16 = 510;
    pub const ray_hint_shadow: u16 = 511;
    pub const stick_valence_scale: u16 = 512;
    pub const seq_view_alignment: u16 = 513;
    pub const seq_view_unaligned_mode: u16 = 514;
    pub const seq_view_unaligned_color: u16 = 515;
    pub const seq_view_fill_char: u16 = 516;
    pub const seq_view_fill_color: u16 = 517;
    pub const seq_view_label_color: u16 = 518;
    pub const surface_carve_normal_cutoff: u16 = 519;
    pub const trace_atoms_mode: u16 = 520;
    pub const session_changed: u16 = 521;
    pub const ray_clip_shadows: u16 = 522;
    pub const mouse_wheel_scale: u16 = 523;
    pub const nonbonded_transparency: u16 = 524;
    pub const ray_spec_local: u16 = 525;
    pub const line_color: u16 = 526;
    pub const ray_label_specular: u16 = 527;
    pub const mesh_skip: u16 = 528;
    pub const label_digits: u16 = 529;
    pub const label_distance_digits: u16 = 530;
    pub const label_angle_digits: u16 = 531;
    pub const label_dihedral_digits: u16 = 532;
    pub const surface_negative_visible: u16 = 533;
    pub const surface_negative_color: u16 = 534;
    pub const mesh_negative_visible: u16 = 535;
    pub const mesh_negative_color: u16 = 536;
    pub const group_auto_mode: u16 = 537;
    pub const group_full_member_names: u16 = 538;
    pub const gradient_max_length: u16 = 539;
    pub const gradient_min_length: u16 = 540;
    pub const gradient_min_slope: u16 = 541;
    pub const gradient_normal_min_dot: u16 = 542;
    pub const gradient_step_size: u16 = 543;
    pub const gradient_spacing: u16 = 544;
    pub const gradient_symmetry: u16 = 545;
    pub const ray_trace_color: u16 = 546;
    pub const group_arrow_prefix: u16 = 547;
    pub const suppress_hidden: u16 = 548;
    pub const session_compression: u16 = 549;

    // 550-599
    pub const movie_fps: u16 = 550;
    pub const ray_transparency_oblique: u16 = 551;
    pub const ray_trace_trans_cutoff: u16 = 552;
    pub const ray_trace_persist_cutoff: u16 = 553;
    pub const ray_transparency_oblique_power: u16 = 554;
    pub const ray_scatter: u16 = 555;
    pub const h_bond_from_proton: u16 = 556;
    pub const auto_copy_images: u16 = 557;
    pub const moe_separate_chains: u16 = 558;
    pub const transparency_global_sort: u16 = 559;
    pub const hide_long_bonds: u16 = 560;
    pub const auto_rename_duplicate_objects: u16 = 561;
    pub const pdb_hetatm_guess_valences: u16 = 562;
    pub const ellipsoid_quality: u16 = 563;
    pub const cgo_ellipsoid_quality: u16 = 564;
    pub const movie_animate_by_frame: u16 = 565;
    pub const ramp_blend_nearby_colors: u16 = 566;
    pub const auto_defer_builds: u16 = 567;
    pub const ellipsoid_probability: u16 = 568;
    pub const ellipsoid_scale: u16 = 569;
    pub const ellipsoid_color: u16 = 570;
    pub const ellipsoid_transparency: u16 = 571;
    pub const movie_rock: u16 = 572;
    pub const cache_mode: u16 = 573;
    pub const dash_color: u16 = 574;
    pub const angle_color: u16 = 575;
    pub const dihedral_color: u16 = 576;
    pub const grid_mode: u16 = 577;
    pub const cache_max: u16 = 578;
    pub const grid_slot: u16 = 579;
    pub const grid_max: u16 = 580;
    pub const cartoon_putty_transform: u16 = 581;
    pub const rock: u16 = 582;
    pub const cone_quality: u16 = 583;
    pub const pdb_formal_charges: u16 = 584;
    pub const ati_bugs: u16 = 585;
    pub const geometry_export_mode: u16 = 586;
    pub const mouse_grid: u16 = 587;
    pub const mesh_cutoff: u16 = 588;
    pub const mesh_carve_selection: u16 = 589;
    pub const mesh_carve_state: u16 = 590;
    pub const mesh_carve_cutoff: u16 = 591;
    pub const mesh_clear_selection: u16 = 592;
    pub const mesh_clear_state: u16 = 593;
    pub const mesh_clear_cutoff: u16 = 594;
    pub const mesh_grid_max: u16 = 595;
    pub const session_cache_optimize: u16 = 596;
    pub const sdof_drag_scale: u16 = 597;
    pub const scene_buttons_mode: u16 = 598;
    pub const scene_buttons: u16 = 599;

    // 600-649
    pub const map_auto_expand_sym: u16 = 600;
    pub const image_copy_always: u16 = 601;
    pub const max_ups: u16 = 602;
    pub const auto_overlay: u16 = 603;
    pub const stick_ball_color: u16 = 604;
    pub const stick_h_scale: u16 = 605;
    pub const sculpt_pyra_inv_weight: u16 = 606;
    pub const keep_alive: u16 = 607;
    pub const fit_kabsch: u16 = 608;
    pub const stereo_dynamic_strength: u16 = 609;
    pub const dynamic_width: u16 = 610;
    pub const dynamic_width_factor: u16 = 611;
    pub const dynamic_width_min: u16 = 612;
    pub const dynamic_width_max: u16 = 613;
    pub const draw_mode: u16 = 614;
    pub const clean_electro_mode: u16 = 615;
    pub const valence_mode: u16 = 616;
    pub const show_frame_rate: u16 = 617;
    pub const movie_panel: u16 = 618;
    pub const mouse_z_scale: u16 = 619;
    pub const movie_auto_store: u16 = 620;
    pub const movie_auto_interpolate: u16 = 621;
    pub const movie_panel_row_height: u16 = 622;
    pub const scene_frame_mode: u16 = 623;
    pub const surface_cavity_mode: u16 = 624;
    pub const surface_cavity_radius: u16 = 625;
    pub const surface_cavity_cutoff: u16 = 626;
    pub const motion_power: u16 = 627;
    pub const motion_bias: u16 = 628;
    pub const motion_simple: u16 = 629;
    pub const motion_linear: u16 = 630;
    pub const motion_hand: u16 = 631;
    pub const pdb_ignore_conect: u16 = 632;
    pub const editor_bond_cycle_mode: u16 = 633;
    pub const movie_quality: u16 = 634;
    pub const label_anchor: u16 = 635;
    pub const fetch_host: u16 = 636;
    pub const dynamic_measures: u16 = 637;
    pub const neighbor_cutoff: u16 = 638;
    pub const heavy_neighbor_cutoff: u16 = 639;
    pub const polar_neighbor_cutoff: u16 = 640;
    pub const surface_residue_cutoff: u16 = 641;
    pub const surface_use_shader: u16 = 642;
    pub const cartoon_use_shader: u16 = 643;
    pub const stick_use_shader: u16 = 644;
    pub const line_use_shader: u16 = 645;
    pub const sphere_use_shader: u16 = 646;
    pub const use_shaders: u16 = 647;
    pub const shaders_from_disk: u16 = 648;
    pub const volume_bit_depth: u16 = 649;

    // 650-699
    pub const volume_color: u16 = 650;
    pub const volume_layers: u16 = 651;
    pub const volume_data_range: u16 = 652;
    pub const auto_defer_atom_count: u16 = 653;
    pub const default_refmac_names: u16 = 654;
    pub const default_phenix_names: u16 = 655;
    pub const default_phenix_no_fill_names: u16 = 656;
    pub const default_buster_names: u16 = 657;
    pub const default_fofc_map_rep: u16 = 658;
    pub const default_2fofc_map_rep: u16 = 659;
    pub const atom_type_format: u16 = 660;
    pub const autoclose_dialogs: u16 = 661;
    pub const bg_gradient: u16 = 662;
    pub const bg_rgb_top: u16 = 663;
    pub const bg_rgb_bottom: u16 = 664;
    pub const ray_volume: u16 = 665;
    pub const ribbon_transparency: u16 = 666;
    pub const state_counter_mode: u16 = 667;
    pub const cgo_use_shader: u16 = 668;
    pub const cgo_shader_ub_color: u16 = 669;
    pub const cgo_shader_ub_normal: u16 = 670;
    pub const cgo_lighting: u16 = 671;
    pub const mesh_use_shader: u16 = 672;
    pub const stick_debug: u16 = 673;
    pub const cgo_debug: u16 = 674;
    pub const stick_round_nub: u16 = 675;
    pub const stick_good_geometry: u16 = 676;
    pub const stick_as_cylinders: u16 = 677;
    pub const mesh_as_cylinders: u16 = 678;
    pub const line_as_cylinders: u16 = 679;
    pub const ribbon_as_cylinders: u16 = 680;
    pub const ribbon_use_shader: u16 = 681;
    pub const excl_display_lists_shaders: u16 = 682;
    pub const dash_use_shader: u16 = 683;
    pub const dash_as_cylinders: u16 = 684;
    pub const nonbonded_use_shader: u16 = 685;
    pub const nonbonded_as_cylinders: u16 = 686;
    pub const cylinders_shader_filter_faces: u16 = 687;
    pub const nb_spheres_size: u16 = 688;
    pub const nb_spheres_quality: u16 = 689;
    pub const nb_spheres_use_shader: u16 = 690;
    pub const render_as_cylinders: u16 = 691;
    pub const alignment_as_cylinders: u16 = 692;
    pub const cartoon_nucleic_acid_as_cylinders: u16 = 693;
    pub const cgo_shader_ub_flags: u16 = 694;
    pub const antialias_shader: u16 = 695;
    pub const offscreen_rendering_multiplier: u16 = 696;
    pub const cylinder_shader_ff_workaround: u16 = 697;
    pub const surface_color_smoothing: u16 = 698;
    pub const surface_color_smoothing_threshold: u16 = 699;

    // 700-749
    pub const dot_use_shader: u16 = 700;
    pub const dot_as_spheres: u16 = 701;
    pub const ambient_occlusion_mode: u16 = 702;
    pub const ambient_occlusion_scale: u16 = 703;
    pub const ambient_occlusion_smooth: u16 = 704;
    pub const smooth_half_bonds: u16 = 705;
    pub const anaglyph_mode: u16 = 706;
    pub const edit_light: u16 = 707;
    pub const suspend_undo: u16 = 708;
    pub const suspend_undo_atom_count: u16 = 709;
    pub const suspend_deferred: u16 = 710;
    pub const pick_surface: u16 = 711;
    pub const bg_image_filename: u16 = 712;
    pub const bg_image_mode: u16 = 713;
    pub const bg_image_tilesize: u16 = 714;
    pub const bg_image_linear: u16 = 715;
    pub const load_object_props_default: u16 = 716;
    pub const load_atom_props_default: u16 = 717;
    pub const label_placement_offset: u16 = 718;
    pub const pdb_conect_nodup: u16 = 719;
    pub const label_connector: u16 = 720;
    pub const label_connector_mode: u16 = 721;
    pub const label_connector_color: u16 = 722;
    pub const label_connector_width: u16 = 723;
    pub const label_connector_ext_length: u16 = 724;
    pub const label_bg_color: u16 = 725;
    pub const use_geometry_shaders: u16 = 726;
    pub const label_relative_mode: u16 = 727;
    pub const label_screen_point: u16 = 728;
    pub const label_multiline_spacing: u16 = 729;
    pub const label_multiline_justification: u16 = 730;
    pub const label_padding: u16 = 731;
    pub const label_bg_transparency: u16 = 732;
    pub const label_bg_outline: u16 = 733;
    pub const ray_label_connector_flat: u16 = 734;
    pub const dash_transparency: u16 = 735;
    pub const pick_labels: u16 = 736;
    pub const label_z_target: u16 = 737;
    pub const session_embeds_data: u16 = 738;
    pub const volume_mode: u16 = 739;
    pub const trilines: u16 = 740;
    pub const collada_export_lighting: u16 = 741;
    pub const collada_geometry_mode: u16 = 742;
    pub const precomputed_lighting: u16 = 743;
    pub const chromadepth: u16 = 744;
    pub const pse_export_version: u16 = 745;
    pub const cif_use_auth: u16 = 746;
    pub const assembly: u16 = 747;
    pub const cif_keepinmemory: u16 = 748;
    pub const pse_binary_dump: u16 = 749;

    // 750-797
    pub const cartoon_gap_cutoff: u16 = 750;
    pub const ignore_case_chain: u16 = 751;
    pub const valence_zero_scale: u16 = 752;
    pub const valence_zero_mode: u16 = 753;
    pub const auto_show_classified: u16 = 754;
    pub const collada_background_box: u16 = 755;
    pub const pick32bit: u16 = 756;
    pub const cartoon_all_alt: u16 = 757;
    pub const display_scale_factor: u16 = 758;
    pub const pick_shading: u16 = 759;
    pub const fetch_type_default: u16 = 760;
    pub const editor_auto_measure: u16 = 761;
    pub const surface_smooth_edges: u16 = 762;
    pub const chem_comp_cartn_use: u16 = 763;
    pub const colored_feedback: u16 = 764;
    pub const sdf_write_zero_order_bonds: u16 = 765;
    pub const cif_metalc_as_zero_order_bonds: u16 = 766;
    pub const seq_view_gap_mode: u16 = 767;
    pub const internal_gui_name_color_mode: u16 = 768;
    pub const openvr_gui_fov: u16 = 769;
    pub const openvr_gui_alpha: u16 = 770;
    pub const openvr_gui_use_alpha: u16 = 771;
    pub const openvr_gui_scene_color: u16 = 772;
    pub const openvr_gui_scene_alpha: u16 = 773;
    pub const openvr_gui_back_color: u16 = 774;
    pub const openvr_gui_back_alpha: u16 = 775;
    pub const openvr_gui_use_backdrop: u16 = 776;
    pub const openvr_gui_overlay: u16 = 777;
    pub const openvr_gui_text: u16 = 778;
    pub const openvr_disable_clipping: u16 = 779;
    pub const openvr_near_plane: u16 = 780;
    pub const openvr_far_plane: u16 = 781;
    pub const openvr_cut_laser: u16 = 782;
    pub const openvr_laser_width: u16 = 783;
    pub const openvr_gui_distance: u16 = 784;
    pub const cartoon_smooth_cylinder_cycles: u16 = 785;
    pub const cartoon_smooth_cylinder_window: u16 = 786;
    pub const isosurface_algorithm: u16 = 787;
    pub const cell_centered: u16 = 788;
    pub const halogen_bond_distance: u16 = 789;
    pub const halogen_bond_as_donor_min_donor_angle: u16 = 790;
    pub const halogen_bond_as_donor_min_acceptor_angle: u16 = 791;
    pub const halogen_bond_as_acceptor_min_donor_angle: u16 = 792;
    pub const halogen_bond_as_acceptor_min_acceptor_angle: u16 = 793;
    pub const halogen_bond_as_acceptor_max_acceptor_angle: u16 = 794;
    pub const salt_bridge_distance: u16 = 795;
    pub const use_tessellation_shaders: u16 = 796;
    pub const cell_color: u16 = 797;

    // 798-801: Silhouette settings
    pub const silhouettes: u16 = 798;
    pub const silhouette_width: u16 = 799;
    pub const silhouette_color: u16 = 800;
    pub const silhouette_depth_jump: u16 = 801;

    // 802-806: Shading mode and Skripkin AO controls.
    pub const shading_mode: u16 = 802;
    pub const skripkin_directions: u16 = 803;
    pub const skripkin_map_size: u16 = 804;
    pub const skripkin_bias: u16 = 805;
    pub const skripkin_intensity: u16 = 806;

    // 807: Surface per-chain
    pub const surface_individual_chains: u16 = 807;
    pub const transparent_panels: u16 = 808;

    // 809-811: Full shading mode (directional light shadows)
    pub const shadow_map_size: u16 = 809;
    pub const shadow_bias: u16 = 810;
    pub const shadow_intensity: u16 = 811;
    pub const shadow_pcf: u16 = 812;
}

// =============================================================================
// Setting Definitions
// =============================================================================

/// Get a setting definition by ID
pub fn get_setting(id: u16) -> Option<&'static Setting> {
    SETTINGS.get(id as usize)
}

/// Get a setting ID by name
pub fn get_setting_id(name: &str) -> Option<u16> {
    SETTINGS.iter().find(|s| s.name == name).map(|s| s.id)
}

/// Get the runtime-wired setting names from the typed registry.
///
/// The stable numeric-ID table in this module is compatibility data for legacy
/// session files; command completion and user-facing lists use the typed
/// registry instead.
pub fn setting_names() -> Vec<&'static str> {
    crate::registry::all_descriptors()
        .iter()
        .map(|d| d.name)
        .collect()
}

// Helper macros for defining settings
macro_rules! s_blank {
    ($id:expr, $name:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Blank,
            level: SettingLevel::Unused,
            default: SettingValue::Int(0),
            min: None,
            max: None,
            value_hints: &[],
        }
    };
}

macro_rules! s_bool {
    ($id:expr, $name:expr, $level:expr, $default:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Bool,
            level: $level,
            default: SettingValue::Bool($default),
            min: Some(0.0),
            max: Some(1.0),
            value_hints: &[],
        }
    };
}

macro_rules! s_int {
    ($id:expr, $name:expr, $level:expr, $default:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Int,
            level: $level,
            default: SettingValue::Int($default),
            min: None,
            max: None,
            value_hints: &[],
        }
    };
    ($id:expr, $name:expr, $level:expr, $default:expr, $min:expr, $max:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Int,
            level: $level,
            default: SettingValue::Int($default),
            min: Some($min as f32),
            max: Some($max as f32),
            value_hints: &[],
        }
    };
}

/// Integer setting with named value variants (e.g., shading_mode: "classic" = 0, "skripkin" = 1).
macro_rules! s_int_enum {
    ($id:expr, $name:expr, $level:expr, $default:expr, $min:expr, $max:expr, $hints:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Int,
            level: $level,
            default: SettingValue::Int($default),
            min: Some($min as f32),
            max: Some($max as f32),
            value_hints: $hints,
        }
    };
}

macro_rules! s_float {
    ($id:expr, $name:expr, $level:expr, $default:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Float,
            level: $level,
            default: SettingValue::Float($default),
            min: None,
            max: None,
            value_hints: &[],
        }
    };
    ($id:expr, $name:expr, $level:expr, $default:expr, $min:expr, $max:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Float,
            level: $level,
            default: SettingValue::Float($default),
            min: Some($min),
            max: Some($max),
            value_hints: &[],
        }
    };
}

macro_rules! s_float3 {
    ($id:expr, $name:expr, $level:expr, $x:expr, $y:expr, $z:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Float3,
            level: $level,
            default: SettingValue::Float3([$x, $y, $z]),
            min: None,
            max: None,
            value_hints: &[],
        }
    };
}

macro_rules! s_color {
    ($id:expr, $name:expr, $level:expr, $default:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::Color,
            level: $level,
            default: SettingValue::Color($default),
            min: None,
            max: None,
            value_hints: &[],
        }
    };
}

macro_rules! s_string {
    ($id:expr, $name:expr, $level:expr) => {
        Setting {
            id: $id,
            name: $name,
            setting_type: SettingType::String,
            level: $level,
            default: SettingValue::Int(0),
            min: None,
            max: None,
            value_hints: &[],
        }
    };
}

// Use shorter level aliases
use SettingLevel::*;

/// All setting definitions (indices stable for PSE session format compatibility)
pub static SETTINGS: &[Setting] = &[
    // 0-49
    s_float!(0, "bonding_vdw_cutoff", Unused, 0.2),
    s_float!(1, "min_mesh_spacing", ObjectState, 0.6),
    s_int!(2, "dot_density", ObjectState, 2),
    s_int!(3, "dot_mode", Global, 0),
    s_float!(4, "solvent_radius", ObjectState, 1.4),
    s_int!(5, "sel_counter", Global, 0),
    s_color!(6, "bg_rgb", Global, patinae_color::ColorValue::rgb(0, 0, 0)),
    s_float!(7, "ambient", Global, 0.14),
    s_float!(8, "direct", Global, 0.45),
    s_float!(9, "reflect", Global, 0.45),
    s_float3!(10, "light", Global, -0.4, -0.4, -1.0),
    s_float!(11, "power", Global, 1.0),
    s_int!(12, "antialias", Global, 1),
    s_int!(13, "cavity_cull", ObjectState, 10),
    s_float!(14, "gl_ambient", Unused, 0.12),
    s_bool!(15, "single_image", Global, false),
    s_float!(16, "movie_delay", Global, 30.0),
    s_float!(17, "ribbon_power", ObjectState, 2.0),
    s_float!(18, "ribbon_power_b", ObjectState, 0.5),
    s_int!(19, "ribbon_sampling", ObjectState, 1),
    s_float!(20, "ribbon_radius", ObjectState, 0.0),
    s_float!(21, "stick_radius", Bond, 0.25),
    s_int!(22, "hash_max", Global, 100),
    s_bool!(23, "orthoscopic", Global, false),
    s_float!(24, "spec_reflect", Global, -1.0),
    s_float!(25, "spec_power", Global, -1.0),
    s_float!(26, "sweep_angle", Global, 20.0),
    s_float!(27, "sweep_speed", Global, 0.75),
    s_bool!(28, "dot_hydrogens", ObjectState, true),
    s_float!(29, "dot_radius", ObjectState, 0.0),
    s_bool!(30, "ray_trace_frames", Global, false),
    s_bool!(31, "cache_frames", Global, false),
    s_bool!(32, "trim_dots", ObjectState, true),
    s_int!(33, "cull_spheres", Unused, 0),
    s_float!(34, "test1", Global, 3.0),
    s_float!(35, "test2", Global, -0.5),
    s_float!(36, "surface_best", ObjectState, 0.25),
    s_float!(37, "surface_normal", ObjectState, 0.5),
    s_int!(38, "surface_quality", ObjectState, 0),
    s_bool!(39, "surface_proximity", ObjectState, true),
    s_bool!(40, "normal_workaround", Global, false),
    s_float!(41, "stereo_angle", Global, 2.1),
    s_float!(42, "stereo_shift", Global, 2.0),
    s_bool!(43, "line_smooth", Global, true),
    s_float!(44, "line_width", Bond, 1.49),
    s_bool!(45, "half_bonds", ObjectState, false),
    s_int!(46, "stick_quality", ObjectState, 8, 3, 100),
    s_float!(47, "stick_overlap", ObjectState, 0.2),
    s_float!(48, "stick_nub", ObjectState, 0.7),
    s_bool!(49, "all_states", Object, false),
    // 50-99
    s_bool!(50, "pickable", ObjectState, true),
    s_bool!(51, "auto_show_lines", Global, true),
    s_float!(52, "idle_delay", Global, 1.5),
    s_float!(53, "no_idle", Global, 2000.0),
    s_float!(54, "fast_idle", Global, 10000.0),
    s_float!(55, "slow_idle", Global, 40000.0),
    s_float!(56, "rock_delay", Global, 30.0),
    s_int!(57, "dist_counter", Global, 0),
    s_float!(58, "dash_length", ObjectState, 0.15),
    s_float!(59, "dash_gap", ObjectState, 0.45),
    s_int!(60, "auto_zoom", Global, -1),
    s_int!(61, "overlay", Global, 0),
    s_bool!(62, "text", Global, false),
    s_int!(63, "button_mode", Global, 0),
    s_bool!(64, "valence", Bond, true),
    s_float!(65, "nonbonded_size", ObjectState, 0.25),
    s_color!(66, "label_color", Atom, patinae_color::ColorValue::INHERIT),
    s_float!(67, "ray_trace_fog", Global, -1.0),
    s_float!(68, "spheroid_scale", ObjectState, 1.0),
    s_float!(69, "ray_trace_fog_start", Global, -1.0),
    s_float!(70, "spheroid_smooth", Global, 1.1),
    s_float!(71, "spheroid_fill", Global, 1.3),
    s_bool!(72, "auto_show_nonbonded", Global, true),
    s_bool!(73, "cache_display", Global, true),
    s_float!(74, "mesh_radius", ObjectState, 0.0),
    s_bool!(75, "backface_cull", Global, false),
    s_float!(76, "gamma", Global, 1.0),
    s_float!(77, "dot_width", ObjectState, 2.0),
    s_bool!(78, "auto_show_selections", Global, true),
    s_bool!(79, "auto_hide_selections", Global, true),
    s_float!(80, "selection_width", Global, 1.0),
    s_float!(81, "selection_overlay", Global, 1.0),
    s_bool!(82, "static_singletons", Object, true),
    s_blank!(83, "unused_83"),
    s_bool!(84, "depth_cue", Global, true),
    s_float!(85, "specular", Global, 1.0),
    s_float!(86, "shininess", Global, 55.0),
    s_int!(87, "sphere_quality", ObjectState, 1, 0, 4),
    s_float!(88, "fog", Global, 1.0),
    s_bool!(89, "isomesh_auto_state", Global, false),
    s_float!(90, "mesh_width", ObjectState, 1.0),
    s_int!(91, "cartoon_sampling", ObjectState, -1),
    s_float!(92, "cartoon_loop_radius", ObjectState, 0.2),
    s_float!(93, "cartoon_loop_quality", ObjectState, -1.0),
    s_float!(94, "cartoon_power", ObjectState, 2.0),
    s_float!(95, "cartoon_power_b", ObjectState, 0.52),
    s_float!(96, "cartoon_rect_length", ObjectState, 1.4),
    s_float!(97, "cartoon_rect_width", ObjectState, 0.4),
    s_int!(98, "internal_gui_width", Global, 220),
    s_bool!(99, "internal_gui", Object, true),
    // 100-149
    s_float!(100, "cartoon_oval_length", ObjectState, 1.35),
    s_float!(101, "cartoon_oval_width", ObjectState, 0.25),
    s_float!(102, "cartoon_oval_quality", ObjectState, -1.0),
    s_float!(103, "cartoon_tube_radius", ObjectState, 0.5),
    s_float!(104, "cartoon_tube_quality", ObjectState, -1.0),
    s_int!(105, "cartoon_debug", ObjectState, 0),
    s_float!(106, "ribbon_width", ObjectState, 3.0),
    s_float!(107, "dash_width", ObjectState, 2.5),
    s_float!(108, "dash_radius", ObjectState, 0.0),
    s_float!(109, "cgo_ray_width_scale", ObjectState, -0.15),
    s_float!(110, "line_radius", ObjectState, 0.0),
    s_bool!(111, "cartoon_round_helices", ObjectState, true),
    s_int!(112, "cartoon_refine_normals", ObjectState, -1),
    s_bool!(113, "cartoon_flat_sheets", ObjectState, true),
    s_bool!(114, "cartoon_smooth_loops", ObjectState, false),
    s_float!(115, "cartoon_dumbbell_length", ObjectState, 1.6),
    s_float!(116, "cartoon_dumbbell_width", ObjectState, 0.17),
    s_float!(117, "cartoon_dumbbell_radius", ObjectState, 0.16),
    s_bool!(118, "cartoon_fancy_helices", ObjectState, false),
    s_bool!(119, "cartoon_fancy_sheets", ObjectState, true),
    s_bool!(120, "ignore_pdb_segi", Global, false),
    s_float!(121, "ribbon_throw", ObjectState, 1.35),
    s_float!(122, "cartoon_throw", ObjectState, 1.35),
    s_int!(123, "cartoon_refine", ObjectState, 5),
    s_int!(124, "cartoon_refine_tips", ObjectState, 10),
    s_bool!(125, "cartoon_discrete_colors", ObjectState, false),
    s_int!(126, "normalize_ccp4_maps", Global, 1),
    s_float!(127, "surface_poor", ObjectState, 0.85),
    s_int!(128, "internal_feedback", Global, 1),
    s_float!(129, "cgo_line_width", ObjectState, 1.0),
    s_float!(130, "cgo_line_radius", ObjectState, -0.05),
    s_int!(131, "logging", Global, 0, 0, 2),
    s_bool!(132, "robust_logs", Global, false),
    s_bool!(133, "log_box_selections", Global, true),
    s_bool!(134, "log_conformations", Global, true),
    s_float!(135, "valence_size", ObjectState, 0.06),
    s_float!(136, "surface_miserable", ObjectState, 2.0),
    s_int!(137, "ray_opaque_background", Global, -1),
    s_float!(138, "surface_transparency", Atom, 0.0),
    s_int!(139, "ray_texture", ObjectState, 0),
    s_float3!(140, "ray_texture_settings", ObjectState, 0.1, 5.0, 1.0),
    s_bool!(141, "suspend_updates", Global, false),
    s_bool!(142, "full_screen", Unused, false),
    s_int!(143, "surface_mode", ObjectState, 0),
    s_color!(
        144,
        "surface_color",
        Atom,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(145, "mesh_mode", ObjectState, 0),
    s_color!(146, "mesh_color", Atom, patinae_color::ColorValue::INHERIT),
    s_bool!(147, "auto_indicate_flags", Global, false),
    s_int!(148, "surface_debug", Global, 0),
    s_float!(149, "ray_improve_shadows", Global, 0.1),
    // 150-199
    s_bool!(150, "smooth_color_triangle", Global, false),
    s_int!(151, "ray_default_renderer", Global, 0),
    s_float!(152, "field_of_view", Global, 20.0),
    s_float!(153, "reflect_power", Global, 1.0),
    s_bool!(154, "preserve_chempy_ids", Global, false),
    s_float!(155, "sphere_scale", Atom, 1.0),
    s_int!(156, "two_sided_lighting", ObjectState, -1),
    s_int!(157, "secondary_structure", Global, 2, 1, 4),
    s_bool!(158, "auto_remove_hydrogens", Global, false),
    s_bool!(159, "raise_exceptions", Unused, true),
    s_bool!(160, "stop_on_exceptions", Global, false),
    s_bool!(161, "sculpting", ObjectState, false),
    s_bool!(162, "auto_sculpt", Global, false),
    s_float!(163, "sculpt_vdw_scale", ObjectState, 0.97),
    s_float!(164, "sculpt_vdw_scale14", ObjectState, 0.9),
    s_float!(165, "sculpt_vdw_weight", ObjectState, 1.0),
    s_float!(166, "sculpt_vdw_weight14", ObjectState, 0.2),
    s_float!(167, "sculpt_bond_weight", ObjectState, 2.25),
    s_float!(168, "sculpt_angl_weight", ObjectState, 1.0),
    s_float!(169, "sculpt_pyra_weight", ObjectState, 1.0),
    s_float!(170, "sculpt_plan_weight", ObjectState, 1.0),
    s_int!(171, "sculpting_cycles", Object, 10),
    s_float!(172, "sphere_transparency", Atom, 0.0),
    s_color!(
        173,
        "sphere_color",
        Atom,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(174, "sculpt_field_mask", ObjectState, 0x1FF),
    s_float!(175, "sculpt_hb_overlap", ObjectState, 1.0),
    s_float!(176, "sculpt_hb_overlap_base", ObjectState, 0.35),
    s_bool!(177, "legacy_vdw_radii", Unused, false),
    s_bool!(178, "sculpt_memory", ObjectState, true),
    s_int!(179, "connect_mode", Global, 0),
    s_int!(180, "cartoon_cylindrical_helices", ObjectState, 0, 0, 2),
    s_float!(181, "cartoon_helix_radius", ObjectState, 2.25),
    s_float!(182, "connect_cutoff", Global, 0.35),
    s_bool!(183, "save_pdb_ss", Unused, false),
    s_float!(184, "sculpt_line_weight", ObjectState, 1.0),
    s_int!(185, "fit_iterations", Global, 1000),
    s_float!(186, "fit_tolerance", Global, 0.0000001),
    s_string!(187, "batch_prefix", Global),
    s_int!(188, "stereo_mode", Global, 2, 1, 13),
    s_int!(189, "cgo_sphere_quality", Global, 1, 0, 4),
    s_bool!(190, "pdb_literal_names", Global, false),
    s_bool!(191, "wrap_output", Global, false),
    s_float!(192, "fog_start", Global, 0.45),
    s_int!(193, "state", Object, 1),
    s_int!(194, "frame", Global, 1),
    s_bool!(195, "ray_shadow", Global, true),
    s_int!(196, "ribbon_trace_atoms", Atom, 0),
    s_int!(197, "security", Global, 1),
    s_float!(198, "stick_transparency", Bond, 0.0),
    s_bool!(199, "ray_transparency_shadows", Global, true),
    // 200-249
    s_int!(200, "session_version_check", Global, 0),
    s_float!(201, "ray_transparency_specular", Global, 0.6),
    s_bool!(202, "stereo_double_pump_mono", Global, false),
    s_bool!(203, "sphere_solvent", ObjectState, false),
    s_int!(204, "mesh_quality", ObjectState, 2),
    s_int!(205, "mesh_solvent", ObjectState, 0),
    s_bool!(206, "dot_solvent", ObjectState, false),
    s_float!(207, "ray_shadow_fudge", Global, 0.001),
    s_float!(208, "ray_triangle_fudge", Global, 0.0000001),
    s_int!(209, "debug_pick", Global, 0),
    s_color!(210, "dot_color", Atom, patinae_color::ColorValue::INHERIT),
    s_float!(211, "mouse_limit", Global, 100.0),
    s_float!(212, "mouse_scale", Global, 1.3),
    s_int!(213, "transparency_mode", ObjectState, 2),
    s_bool!(214, "clamp_colors", Global, true),
    s_float!(215, "pymol_space_max_red", Global, 0.9),
    s_float!(216, "pymol_space_max_green", Global, 0.75),
    s_float!(217, "pymol_space_max_blue", Global, 0.9),
    s_float!(218, "pymol_space_min_factor", Global, 0.15),
    s_bool!(219, "roving_origin", Global, true),
    s_float!(220, "roving_lines", Global, 10.0),
    s_float!(221, "roving_sticks", Global, 6.0),
    s_float!(222, "roving_spheres", Global, 0.0),
    s_float!(223, "roving_labels", Global, 0.0),
    s_float!(224, "roving_delay", Global, 0.2),
    s_string!(225, "roving_selection", Global),
    s_bool!(226, "roving_byres", Global, true),
    s_float!(227, "roving_ribbon", Global, -7.0),
    s_float!(228, "roving_cartoon", Global, 0.0),
    s_float!(229, "roving_polar_contacts", Global, 7.0),
    s_float!(230, "roving_polar_cutoff", Global, 3.31),
    s_float!(231, "roving_nonbonded", Global, 0.0),
    s_int!(232, "float_labels", ObjectState, 0),
    s_bool!(233, "roving_detail", Global, false),
    s_float!(234, "roving_nb_spheres", Global, 8.0),
    s_color!(
        235,
        "ribbon_color",
        Atom,
        patinae_color::ColorValue::INHERIT
    ),
    s_color!(
        236,
        "cartoon_color",
        Atom,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(237, "ribbon_smooth", Unused, 0),
    s_bool!(238, "auto_color", Global, true),
    s_int!(239, "auto_color_next", Global, 0),
    s_color!(
        240,
        "ray_interior_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_color!(
        241,
        "cartoon_highlight_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(242, "coulomb_units_factor", Global, 557.0),
    s_float!(243, "coulomb_dielectric", Global, 2.0),
    s_bool!(244, "ray_interior_shadows", Global, false),
    s_int!(245, "ray_interior_texture", Global, -1),
    s_string!(246, "roving_map1_name", Global),
    s_string!(247, "roving_map2_name", Global),
    s_string!(248, "roving_map3_name", Global),
    s_float!(249, "roving_map1_level", Global, 1.0),
    // 250-299
    s_float!(250, "roving_map2_level", Global, 2.0),
    s_float!(251, "roving_map3_level", Global, 3.0),
    s_float!(252, "roving_isomesh", Global, 8.0),
    s_float!(253, "roving_isosurface", Global, 0.0),
    s_bool!(254, "scenes_changed", Global, true),
    s_float!(255, "gaussian_b_adjust", Global, 0.0),
    s_bool!(256, "pdb_standard_order", Global, true),
    s_int!(257, "cartoon_smooth_first", ObjectState, 1),
    s_int!(258, "cartoon_smooth_last", ObjectState, 1),
    s_int!(259, "cartoon_smooth_cycles", ObjectState, 2),
    s_int!(260, "cartoon_flat_cycles", ObjectState, 4),
    s_int!(261, "max_threads", Object, 1),
    s_int!(262, "show_progress", Global, 1),
    s_int!(263, "use_display_lists", Unused, 0),
    s_int!(264, "cache_memory", Global, 0),
    s_bool!(265, "simplify_display_lists", Unused, false),
    s_int!(266, "retain_order", Object, 0),
    s_int!(267, "pdb_hetatm_sort", Object, 0),
    s_int!(268, "pdb_use_ter_records", Global, 1),
    s_int!(269, "cartoon_trace_atoms", Atom, 0),
    s_int!(270, "ray_oversample_cutoff", Global, 120),
    s_float!(271, "gaussian_resolution", Global, 2.0),
    s_float!(272, "gaussian_b_floor", Global, 0.0),
    s_int!(273, "sculpt_nb_interval", ObjectState, 17),
    s_float!(274, "sculpt_tors_weight", ObjectState, 0.05),
    s_float!(275, "sculpt_tors_tolerance", ObjectState, 0.05),
    s_bool!(276, "stick_ball", Atom, false),
    s_float!(277, "stick_ball_ratio", ObjectState, 1.0),
    s_bool!(278, "stick_fixed_radius", ObjectState, false),
    s_float!(279, "cartoon_transparency", Atom, 0.0),
    s_bool!(280, "dash_round_ends", ObjectState, true),
    s_float!(281, "h_bond_max_angle", Global, 63.0),
    s_float!(282, "h_bond_cutoff_center", Global, 3.6),
    s_float!(283, "h_bond_cutoff_edge", Global, 3.2),
    s_float!(284, "h_bond_power_a", Global, 1.6),
    s_float!(285, "h_bond_power_b", Global, 5.0),
    s_float!(286, "h_bond_cone", Global, 180.0),
    s_float!(287, "ss_helix_psi_target", Global, -48.0),
    s_float!(288, "ss_helix_psi_include", Global, 55.0),
    s_float!(289, "ss_helix_psi_exclude", Global, 85.0),
    s_float!(290, "ss_helix_phi_target", Global, -57.0),
    s_float!(291, "ss_helix_phi_include", Global, 55.0),
    s_float!(292, "ss_helix_phi_exclude", Global, 85.0),
    s_float!(293, "ss_strand_psi_target", Global, 124.0),
    s_float!(294, "ss_strand_psi_include", Global, 40.0),
    s_float!(295, "ss_strand_psi_exclude", Global, 90.0),
    s_float!(296, "ss_strand_phi_target", Global, -129.0),
    s_float!(297, "ss_strand_phi_include", Global, 40.0),
    s_float!(298, "ss_strand_phi_exclude", Global, 100.0),
    s_bool!(299, "movie_loop", Object, true),
    // 300-349
    s_bool!(300, "pdb_retain_ids", Global, false),
    s_bool!(301, "pdb_no_end_record", Global, false),
    s_float!(302, "cgo_dot_width", ObjectState, 2.0),
    s_float!(303, "cgo_dot_radius", ObjectState, -1.0),
    s_bool!(304, "defer_updates", Global, false),
    s_bool!(305, "normalize_o_maps", Global, true),
    s_bool!(306, "swap_dsn6_bytes", Global, true),
    s_bool!(307, "pdb_insertions_go_first", Global, false),
    s_bool!(308, "roving_origin_z", Global, true),
    s_float!(309, "roving_origin_z_cushion", Global, 3.0),
    s_float!(310, "specular_intensity", Global, 0.5),
    s_int!(311, "overlay_lines", Global, 5),
    s_float!(312, "ray_transparency_spec_cut", Global, 0.9),
    s_bool!(313, "internal_prompt", Global, true),
    s_bool!(314, "normalize_grd_maps", Global, false),
    s_bool!(315, "ray_blend_colors", Global, false),
    s_float!(316, "ray_blend_red", Global, 0.17),
    s_float!(317, "ray_blend_green", Global, 0.25),
    s_float!(318, "ray_blend_blue", Global, 0.14),
    s_float!(319, "png_screen_gamma", Global, 2.4),
    s_float!(320, "png_file_gamma", Global, 1.0),
    s_bool!(321, "editor_label_fragments", Global, false),
    s_int!(322, "internal_gui_control_size", Global, 18),
    s_bool!(323, "auto_dss", Global, true),
    s_int!(324, "transparency_picking_mode", ObjectState, 2),
    s_int!(325, "virtual_trackball", Global, 1),
    s_int!(326, "pdb_reformat_names_mode", Global, 0, 0, 4),
    s_float!(327, "ray_pixel_scale", Global, 1.3),
    s_int!(328, "label_font_id", ObjectState, 5),
    s_bool!(329, "pdb_conect_all", Global, false),
    s_string!(330, "button_mode_name", Global),
    s_int!(331, "surface_type", ObjectState, 0),
    s_bool!(332, "dot_normals", ObjectState, true),
    s_bool!(333, "session_migration", Global, true),
    s_bool!(334, "mesh_normals", ObjectState, true),
    s_int!(335, "mesh_type", ObjectState, 0, 0, 1),
    s_bool!(336, "dot_lighting", ObjectState, true),
    s_bool!(337, "mesh_lighting", ObjectState, false),
    s_bool!(338, "surface_solvent", ObjectState, false),
    s_int!(339, "triangle_max_passes", Global, 5),
    s_float!(340, "ray_interior_reflect", Global, 0.4),
    s_int!(341, "internal_gui_mode", Global, 0),
    s_string!(342, "surface_carve_selection", ObjectState),
    s_int!(343, "surface_carve_state", ObjectState, 0),
    s_float!(344, "surface_carve_cutoff", ObjectState, 0.0),
    s_string!(345, "surface_clear_selection", ObjectState),
    s_int!(346, "surface_clear_state", ObjectState, 0),
    s_float!(347, "surface_clear_cutoff", ObjectState, 0.0),
    s_float!(348, "surface_trim_cutoff", ObjectState, 0.2),
    s_float!(349, "surface_trim_factor", ObjectState, 2.0),
    // 350-399
    s_int!(350, "ray_max_passes", Global, 25),
    s_bool!(351, "active_selections", Global, true),
    s_float!(352, "ray_transparency_contrast", Global, 1.0),
    s_bool!(353, "seq_view", Object, false),
    // mouse_selection_mode: 0 = atoms, 1 = residues, 2 = chains, 3 = segments, 4 = objects, 5 = molecules, 6 = C-alphas
    s_int_enum!(
        354,
        "mouse_selection_mode",
        Global,
        1,
        0,
        6,
        &[
            ("atoms", SettingValue::Int(0)),
            ("residues", SettingValue::Int(1)),
            ("chains", SettingValue::Int(2)),
            ("segments", SettingValue::Int(3)),
            ("objects", SettingValue::Int(4)),
            ("molecules", SettingValue::Int(5)),
            ("c_alphas", SettingValue::Int(6))
        ]
    ),
    s_int!(355, "seq_view_label_spacing", Object, 5),
    s_int!(356, "seq_view_label_start", Object, 1),
    s_int!(357, "seq_view_format", Object, 0),
    s_int!(358, "seq_view_location", Global, 0),
    s_bool!(359, "seq_view_overlay", Global, false),
    s_bool!(360, "auto_classify_atoms", Global, true),
    s_int!(361, "cartoon_nucleic_acid_mode", ObjectState, 4),
    s_color!(
        362,
        "seq_view_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(363, "seq_view_label_mode", Global, 2),
    s_int!(364, "surface_ramp_above_mode", ObjectState, 0),
    s_bool!(365, "stereo", Global, false),
    s_int!(366, "wizard_prompt_mode", Global, 1),
    s_float!(367, "coulomb_cutoff", Global, 10.0),
    s_bool!(368, "slice_track_camera", Object, false),
    s_float!(369, "slice_height_scale", Object, 1.0),
    s_bool!(370, "slice_height_map", Object, false),
    s_float!(371, "slice_grid", Object, 0.3),
    s_bool!(372, "slice_dynamic_grid", Object, false),
    s_float!(373, "slice_dynamic_grid_resolution", Object, 3.0),
    s_bool!(374, "pdb_insure_orthogonal", Global, true),
    s_float!(375, "ray_direct_shade", Global, 0.0),
    s_color!(376, "stick_color", Bond, patinae_color::ColorValue::INHERIT),
    s_float!(377, "cartoon_putty_radius", ObjectState, 0.4),
    s_float!(378, "cartoon_putty_quality", ObjectState, -1.0),
    s_float!(379, "cartoon_putty_scale_min", ObjectState, 0.6),
    s_float!(380, "cartoon_putty_scale_max", ObjectState, 4.0),
    s_float!(381, "cartoon_putty_scale_power", ObjectState, 1.5),
    s_float!(382, "cartoon_putty_range", ObjectState, 2.0),
    s_bool!(383, "cartoon_side_chain_helper", Atom, false),
    s_bool!(384, "surface_optimize_subsets", ObjectState, true),
    s_int!(385, "multiplex", Global, -1),
    s_bool!(386, "texture_fonts", Unused, false),
    s_bool!(387, "pqr_no_chain_id", Global, true),
    s_bool!(388, "animation", Global, true),
    s_float!(389, "animation_duration", Global, 0.75),
    s_int!(390, "scene_animation", Global, -1),
    s_bool!(391, "line_stick_helper", ObjectState, true),
    s_int!(392, "ray_orthoscopic", Global, -1),
    s_int!(393, "ribbon_side_chain_helper", Atom, 0),
    s_float!(394, "selection_width_max", Global, 10.0),
    s_float!(395, "selection_width_scale", Global, 2.0),
    s_string!(396, "scene_current_name", Global),
    s_bool!(397, "presentation", Global, false),
    s_int!(398, "presentation_mode", Global, 1),
    s_bool!(399, "pdb_truncate_residue_name", Global, false),
    // 400-449
    s_bool!(400, "scene_loop", Global, false),
    s_int!(401, "sweep_mode", Global, 0),
    s_float!(402, "sweep_phase", Global, 0.0),
    s_bool!(403, "scene_restart_movie_delay", Global, true),
    s_bool!(404, "mouse_restart_movie_delay", Global, false),
    s_float!(405, "angle_size", ObjectState, 0.6666),
    s_float!(406, "angle_label_position", ObjectState, 0.5),
    s_float!(407, "dihedral_size", ObjectState, 0.6666),
    s_float!(408, "dihedral_label_position", ObjectState, 1.2),
    s_int!(409, "defer_builds_mode", ObjectState, 0),
    s_bool!(410, "seq_view_discrete_by_state", Object, true),
    s_float!(411, "scene_animation_duration", Global, 2.25),
    s_string!(412, "wildcard", Object),
    s_string!(413, "atom_name_wildcard", Object),
    s_bool!(414, "ignore_case", Global, true),
    s_bool!(415, "presentation_auto_quit", Global, true),
    s_bool!(416, "editor_auto_dihedral", Global, true),
    s_bool!(417, "presentation_auto_start", Global, true),
    s_bool!(418, "validate_object_names", Global, true),
    s_bool!(419, "unused_boolean_def_true", Unused, true),
    s_bool!(420, "auto_show_spheres", Global, false),
    s_int!(421, "sphere_mode", ObjectState, 9, -1, 11),
    s_float!(422, "sphere_point_max_size", ObjectState, 18.0),
    s_float!(423, "sphere_point_size", Global, 1.0),
    s_bool!(424, "pdb_honor_model_number", Global, false),
    s_bool!(425, "rank_assisted_sorts", Global, true),
    s_int!(426, "ribbon_nucleic_acid_mode", ObjectState, 0),
    s_int!(427, "cartoon_ring_mode", Atom, 0),
    s_float!(428, "cartoon_ring_width", Atom, 0.125),
    s_color!(
        429,
        "cartoon_ring_color",
        Atom,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(430, "cartoon_ring_finder", ObjectState, 1),
    s_int!(431, "cartoon_tube_cap", ObjectState, 2),
    s_int!(432, "cartoon_loop_cap", ObjectState, 1),
    s_int!(433, "nvidia_bugs", Global, 0),
    s_float!(434, "image_dots_per_inch", Global, 0.0),
    s_bool!(435, "opaque_background", Global, false),
    s_bool!(436, "draw_frames", Global, false),
    s_bool!(437, "show_alpha_checker", Global, true),
    s_int!(438, "matrix_mode", ObjectState, -1, -1, 2),
    s_bool!(439, "editor_auto_origin", Global, true),
    s_string!(440, "session_file", Global),
    s_float!(441, "cgo_transparency", ObjectState, 0.0),
    s_bool!(442, "legacy_mouse_zoom", Global, false),
    s_bool!(443, "auto_number_selections", Global, false),
    s_int!(444, "sculpt_vdw_vis_mode", ObjectState, 0),
    s_float!(445, "sculpt_vdw_vis_min", ObjectState, -0.1),
    s_float!(446, "sculpt_vdw_vis_mid", ObjectState, 0.1),
    s_float!(447, "sculpt_vdw_vis_max", ObjectState, 0.3),
    s_int!(448, "cartoon_ladder_mode", Atom, 1),
    s_float!(449, "cartoon_ladder_radius", Atom, 0.25),
    // 450-499
    s_color!(
        450,
        "cartoon_ladder_color",
        Atom,
        patinae_color::ColorValue::INHERIT
    ),
    s_color!(
        451,
        "cartoon_nucleic_acid_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(452, "cartoon_ring_transparency", ObjectState, -1.0),
    s_float!(453, "label_size", ObjectState, 14.0),
    s_float!(454, "spec_direct", Global, 0.0),
    s_int!(455, "light_count", Global, 2, 1, 10),
    s_float3!(456, "light2", Global, -0.55, -0.7, 0.15),
    s_float3!(457, "light3", Global, 0.3, -0.6, -0.2),
    s_bool!(458, "hide_underscore_names", Global, true),
    s_bool!(459, "selection_round_points", Global, false),
    s_int!(460, "distance_exclusion", Global, 5),
    s_int!(461, "h_bond_exclusion", Global, 3),
    s_int!(462, "label_shadow_mode", Global, 0),
    s_float3!(463, "light4", Global, -1.2, 0.3, -0.2),
    s_float3!(464, "light5", Global, 0.3, 0.6, -0.75),
    s_float3!(465, "light6", Global, -0.3, 0.5, 0.0),
    s_float3!(466, "light7", Global, 0.9, -0.1, -0.15),
    s_color!(
        467,
        "label_outline_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(468, "ray_trace_mode", Global, 0),
    s_float!(469, "ray_trace_gain", Global, 0.12),
    s_bool!(470, "selection_visible_only", Global, false),
    s_float3!(471, "label_position", AtomState, 0.0, 0.0, 1.75),
    s_float!(472, "ray_trace_depth_factor", Global, 0.1),
    s_float!(473, "ray_trace_slope_factor", Global, 0.6),
    s_float!(474, "ray_trace_disco_factor", Global, 0.05),
    s_float!(475, "ray_shadow_decay_factor", Global, 0.0),
    s_int!(476, "ray_interior_mode", Global, 0),
    s_float!(477, "ray_legacy_lighting", Global, 0.0),
    s_bool!(478, "sculpt_auto_center", Global, false),
    s_int!(479, "pdb_discrete_chains", Global, -1),
    s_int!(480, "pdb_unbond_cations", Global, 1),
    s_float!(481, "sculpt_tri_scale", ObjectState, 1.025),
    s_float!(482, "sculpt_tri_weight", ObjectState, 1.0),
    s_int!(483, "sculpt_tri_min", ObjectState, 2),
    s_int!(484, "sculpt_tri_max", ObjectState, 18),
    s_int!(485, "sculpt_tri_mode", ObjectState, 0),
    s_string!(486, "pdb_echo_tags", Global),
    s_bool!(487, "connect_bonded", Global, false),
    s_float!(488, "spec_direct_power", Global, 55.0),
    s_float3!(489, "light8", Global, 1.3, 2.0, 0.8),
    s_float3!(490, "light9", Global, -1.7, -0.5, 1.2),
    s_float!(491, "ray_shadow_decay_range", Global, 1.8),
    s_int!(492, "spec_count", Global, -1),
    s_float!(493, "sculpt_min_scale", ObjectState, 0.975),
    s_float!(494, "sculpt_min_weight", ObjectState, 0.75),
    s_float!(495, "sculpt_min_min", ObjectState, 4.0),
    s_float!(496, "sculpt_min_max", ObjectState, 12.0),
    s_float!(497, "sculpt_max_scale", ObjectState, 1.025),
    s_float!(498, "sculpt_max_weight", ObjectState, 0.75),
    s_float!(499, "sculpt_max_min", ObjectState, 4.0),
    // 500-549
    s_float!(500, "sculpt_max_max", ObjectState, 12.0),
    s_int!(501, "surface_circumscribe", ObjectState, -1),
    s_float!(502, "sculpt_avd_weight", ObjectState, 4.0),
    s_float!(503, "sculpt_avd_gap", ObjectState, -1.0),
    s_float!(504, "sculpt_avd_range", ObjectState, -1.0),
    s_int!(505, "sculpt_avd_excl", ObjectState, 7),
    s_bool!(506, "async_builds", Object, false),
    s_string!(507, "fetch_path", Global),
    s_float!(508, "cartoon_ring_radius", Atom, -1.0),
    s_bool!(509, "ray_color_ramps", ObjectState, false),
    s_float!(510, "ray_hint_camera", Global, 2.15),
    s_float!(511, "ray_hint_shadow", Global, 0.65),
    s_float!(512, "stick_valence_scale", ObjectState, 1.0),
    s_string!(513, "seq_view_alignment", Global),
    s_int!(514, "seq_view_unaligned_mode", Global, 0),
    s_color!(
        515,
        "seq_view_unaligned_color",
        Global,
        patinae_color::ColorValue::INHERIT
    ),
    s_string!(516, "seq_view_fill_char", Global),
    s_color!(
        517,
        "seq_view_fill_color",
        Global,
        patinae_color::ColorValue::palette(104)
    ),
    s_color!(
        518,
        "seq_view_label_color",
        Global,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(519, "surface_carve_normal_cutoff", ObjectState, -1.0),
    s_int!(520, "trace_atoms_mode", ObjectState, 5),
    s_bool!(521, "session_changed", Global, false),
    s_bool!(522, "ray_clip_shadows", Global, false),
    s_float!(523, "mouse_wheel_scale", Global, 0.5),
    s_float!(524, "nonbonded_transparency", Atom, 0.0),
    s_bool!(525, "ray_spec_local", Global, false),
    s_color!(526, "line_color", Bond, patinae_color::ColorValue::INHERIT),
    s_float!(527, "ray_label_specular", Global, 1.0),
    s_int!(528, "mesh_skip", ObjectState, 0),
    s_int!(529, "label_digits", ObjectState, 1),
    s_int!(530, "label_distance_digits", ObjectState, -1),
    s_int!(531, "label_angle_digits", ObjectState, -1),
    s_int!(532, "label_dihedral_digits", ObjectState, -1),
    s_bool!(533, "surface_negative_visible", Object, false),
    s_color!(
        534,
        "surface_negative_color",
        Object,
        patinae_color::ColorValue::INHERIT
    ),
    s_bool!(535, "mesh_negative_visible", Object, false),
    s_color!(
        536,
        "mesh_negative_color",
        Object,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(537, "group_auto_mode", Global, 1),
    s_int!(538, "group_full_member_names", Global, 0),
    s_float!(539, "gradient_max_length", ObjectState, 100.0),
    s_float!(540, "gradient_min_length", ObjectState, 2.0),
    s_float!(541, "gradient_min_slope", ObjectState, 0.00001),
    s_float!(542, "gradient_normal_min_dot", ObjectState, 0.7),
    s_float!(543, "gradient_step_size", ObjectState, 0.25),
    s_int!(544, "gradient_spacing", ObjectState, 3),
    s_float!(545, "gradient_symmetry", ObjectState, 0.0),
    s_color!(
        546,
        "ray_trace_color",
        Global,
        patinae_color::ColorValue::INHERIT
    ),
    s_bool!(547, "group_arrow_prefix", Global, false),
    s_bool!(548, "suppress_hidden", Global, true),
    s_bool!(549, "session_compression", Global, false),
    // 550-599
    s_float!(550, "movie_fps", Global, 30.0),
    s_float!(551, "ray_transparency_oblique", Global, 0.0),
    s_float!(552, "ray_trace_trans_cutoff", Global, 0.05),
    s_float!(553, "ray_trace_persist_cutoff", Global, 0.1),
    s_float!(554, "ray_transparency_oblique_power", Global, 4.0),
    s_float!(555, "ray_scatter", Global, 0.0),
    s_bool!(556, "h_bond_from_proton", Global, true),
    s_bool!(557, "auto_copy_images", Global, false),
    s_int!(558, "moe_separate_chains", Global, -1),
    s_bool!(559, "transparency_global_sort", Global, false),
    s_bool!(560, "hide_long_bonds", ObjectState, false),
    s_bool!(561, "auto_rename_duplicate_objects", Global, false),
    s_bool!(562, "pdb_hetatm_guess_valences", Global, true),
    s_int!(563, "ellipsoid_quality", Global, 1),
    s_int!(564, "cgo_ellipsoid_quality", Global, -1),
    s_bool!(565, "movie_animate_by_frame", Global, false),
    s_bool!(566, "ramp_blend_nearby_colors", Global, false),
    s_int!(567, "auto_defer_builds", Global, 500),
    s_float!(568, "ellipsoid_probability", ObjectState, 0.5),
    s_float!(569, "ellipsoid_scale", Atom, 1.0),
    s_color!(
        570,
        "ellipsoid_color",
        Atom,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(571, "ellipsoid_transparency", Atom, 0.0),
    s_int!(572, "movie_rock", Global, -1),
    s_int!(573, "cache_mode", ObjectState, 0),
    s_color!(
        574,
        "dash_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_color!(
        575,
        "angle_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_color!(
        576,
        "dihedral_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_int!(577, "grid_mode", Global, 0, 0, 3),
    s_int!(578, "cache_max", Global, 25000000),
    s_int!(579, "grid_slot", Object, -1),
    s_int!(580, "grid_max", Global, -1),
    s_int!(581, "cartoon_putty_transform", ObjectState, 0),
    s_bool!(582, "rock", Global, false),
    s_int!(583, "cone_quality", Global, 18),
    s_bool!(584, "pdb_formal_charges", Global, true),
    s_int!(585, "ati_bugs", Global, 0),
    s_int!(586, "geometry_export_mode", Global, 0),
    s_bool!(587, "mouse_grid", Global, true),
    s_float!(588, "mesh_cutoff", ObjectState, 0.0),
    s_string!(589, "mesh_carve_selection", ObjectState),
    s_int!(590, "mesh_carve_state", ObjectState, 0),
    s_float!(591, "mesh_carve_cutoff", ObjectState, 0.0),
    s_string!(592, "mesh_clear_selection", ObjectState),
    s_int!(593, "mesh_clear_state", ObjectState, 0),
    s_float!(594, "mesh_clear_cutoff", ObjectState, 0.0),
    s_int!(595, "mesh_grid_max", ObjectState, 80),
    s_int!(596, "session_cache_optimize", Global, 0),
    s_float!(597, "sdof_drag_scale", Global, 0.5),
    s_int!(598, "scene_buttons_mode", Unused, 1),
    s_bool!(599, "scene_buttons", Global, true),
    // 600-649
    s_bool!(600, "map_auto_expand_sym", Object, true),
    s_bool!(601, "image_copy_always", Global, false),
    s_int!(602, "max_ups", Global, 0),
    s_int!(603, "auto_overlay", Global, 0),
    s_color!(
        604,
        "stick_ball_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(605, "stick_h_scale", ObjectState, 0.4),
    s_float!(606, "sculpt_pyra_inv_weight", ObjectState, 10.0),
    s_bool!(607, "keep_alive", Global, false),
    s_int!(608, "fit_kabsch", Global, 0),
    s_float!(609, "stereo_dynamic_strength", Global, 0.5),
    s_bool!(610, "dynamic_width", Global, true),
    s_float!(611, "dynamic_width_factor", Global, 0.06),
    s_float!(612, "dynamic_width_min", Global, 0.75),
    s_float!(613, "dynamic_width_max", Global, 2.5),
    s_int!(614, "draw_mode", Global, 0),
    s_int!(615, "clean_electro_mode", Global, 1),
    s_int!(616, "valence_mode", ObjectState, 1),
    s_bool!(617, "show_frame_rate", Global, false),
    s_int!(618, "movie_panel", Global, 1),
    s_float!(619, "mouse_z_scale", Global, 1.0),
    s_bool!(620, "movie_auto_store", Object, true),
    s_bool!(621, "movie_auto_interpolate", Object, true),
    s_int!(622, "movie_panel_row_height", Global, 15),
    s_int!(623, "scene_frame_mode", Global, -1),
    s_int!(624, "surface_cavity_mode", ObjectState, 0),
    s_float!(625, "surface_cavity_radius", ObjectState, 7.0),
    s_float!(626, "surface_cavity_cutoff", ObjectState, -3.0),
    s_float!(627, "motion_power", Object, 0.0),
    s_float!(628, "motion_bias", Object, -1.0),
    s_int!(629, "motion_simple", Object, 0),
    s_float!(630, "motion_linear", Object, 0.0),
    s_int!(631, "motion_hand", Object, 1),
    s_bool!(632, "pdb_ignore_conect", Global, false),
    s_bool!(633, "editor_bond_cycle_mode", Object, true),
    s_int!(634, "movie_quality", Global, 90),
    s_string!(635, "label_anchor", Global),
    s_string!(636, "fetch_host", Global),
    s_bool!(637, "dynamic_measures", Object, true),
    s_float!(638, "neighbor_cutoff", Global, 3.5),
    s_float!(639, "heavy_neighbor_cutoff", Global, 3.5),
    s_float!(640, "polar_neighbor_cutoff", Global, 3.5),
    s_float!(641, "surface_residue_cutoff", Global, 2.5),
    s_bool!(642, "surface_use_shader", Global, true),
    s_bool!(643, "cartoon_use_shader", Global, true),
    s_bool!(644, "stick_use_shader", Global, true),
    s_bool!(645, "line_use_shader", Global, true),
    s_bool!(646, "sphere_use_shader", Global, true),
    s_bool!(647, "use_shaders", ObjectState, false),
    s_bool!(648, "shaders_from_disk", Global, false),
    s_int!(649, "volume_bit_depth", Object, 16),
    // 650-699
    s_color!(
        650,
        "volume_color",
        Unused,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(651, "volume_layers", Object, 256.0),
    s_float!(652, "volume_data_range", Object, 5.0),
    s_int!(653, "auto_defer_atom_count", Global, 0),
    s_string!(654, "default_refmac_names", Global),
    s_string!(655, "default_phenix_names", Global),
    s_string!(656, "default_phenix_no_fill_names", Global),
    s_string!(657, "default_buster_names", Global),
    s_string!(658, "default_fofc_map_rep", Global),
    s_string!(659, "default_2fofc_map_rep", Global),
    s_string!(660, "atom_type_format", Global),
    s_bool!(661, "autoclose_dialogs", Global, true),
    s_int!(662, "bg_gradient", Global, 0),
    s_color!(
        663,
        "bg_rgb_top",
        Global,
        patinae_color::ColorValue::rgb(0, 0, 77)
    ),
    s_color!(
        664,
        "bg_rgb_bottom",
        Global,
        patinae_color::ColorValue::rgb(51, 51, 128)
    ),
    s_bool!(665, "ray_volume", Global, false),
    s_float!(666, "ribbon_transparency", ObjectState, 0.0),
    s_int!(667, "state_counter_mode", Object, -1, -1, 2),
    s_bool!(668, "cgo_use_shader", Global, true),
    s_bool!(669, "cgo_shader_ub_color", Global, false),
    s_bool!(670, "cgo_shader_ub_normal", Global, false),
    s_int!(671, "cgo_lighting", Object, 1),
    s_bool!(672, "mesh_use_shader", Global, true),
    s_int!(673, "stick_debug", Global, 0),
    s_int!(674, "cgo_debug", Global, 0),
    s_int!(675, "stick_round_nub", ObjectState, 0),
    s_int!(676, "stick_good_geometry", ObjectState, 0),
    s_bool!(677, "stick_as_cylinders", Global, true),
    s_bool!(678, "mesh_as_cylinders", Global, false),
    s_bool!(679, "line_as_cylinders", Global, false),
    s_bool!(680, "ribbon_as_cylinders", ObjectState, false),
    s_bool!(681, "ribbon_use_shader", Global, true),
    s_bool!(682, "excl_display_lists_shaders", Unused, false),
    s_bool!(683, "dash_use_shader", Global, true),
    s_bool!(684, "dash_as_cylinders", Global, true),
    s_bool!(685, "nonbonded_use_shader", Global, true),
    s_bool!(686, "nonbonded_as_cylinders", Global, false),
    s_bool!(687, "cylinders_shader_filter_faces", Unused, true),
    s_float!(688, "nb_spheres_size", ObjectState, 0.25),
    s_int!(689, "nb_spheres_quality", ObjectState, 1, 0, 4),
    s_int!(690, "nb_spheres_use_shader", Global, 1, 0, 2),
    s_bool!(691, "render_as_cylinders", Global, true),
    s_bool!(692, "alignment_as_cylinders", Global, false),
    s_int!(693, "cartoon_nucleic_acid_as_cylinders", Global, 1, 0, 3),
    s_bool!(694, "cgo_shader_ub_flags", Global, false),
    s_int!(695, "antialias_shader", Global, 0),
    s_float!(696, "offscreen_rendering_multiplier", Unused, 4.0),
    s_bool!(697, "cylinder_shader_ff_workaround", Unused, true),
    s_int!(698, "surface_color_smoothing", Global, 1),
    s_float!(699, "surface_color_smoothing_threshold", Global, 0.05),
    // 700-749
    s_bool!(700, "dot_use_shader", Global, true),
    s_bool!(701, "dot_as_spheres", ObjectState, false),
    s_int!(702, "ambient_occlusion_mode", ObjectState, 0),
    s_float!(703, "ambient_occlusion_scale", ObjectState, 25.0),
    s_int!(704, "ambient_occlusion_smooth", ObjectState, 10),
    s_bool!(705, "smooth_half_bonds", Global, true),
    s_int!(706, "anaglyph_mode", Global, 4, 0, 4),
    s_int!(707, "edit_light", Global, 1),
    s_bool!(708, "suspend_undo", Object, false),
    s_int!(709, "suspend_undo_atom_count", Global, 1000),
    s_bool!(710, "suspend_deferred", Global, false),
    s_bool!(711, "pick_surface", ObjectState, false),
    s_string!(712, "bg_image_filename", Global),
    s_int!(713, "bg_image_mode", Global, 0, 0, 3),
    s_float3!(714, "bg_image_tilesize", Global, 100.0, 100.0, 0.0),
    s_bool!(715, "bg_image_linear", Global, true),
    s_string!(716, "load_object_props_default", Global),
    s_string!(717, "load_atom_props_default", Global),
    s_float3!(718, "label_placement_offset", AtomState, 0.0, 0.0, 0.0),
    s_bool!(719, "pdb_conect_nodup", Global, true),
    s_bool!(720, "label_connector", AtomState, false),
    s_int!(721, "label_connector_mode", AtomState, 0, 0, 4),
    s_color!(
        722,
        "label_connector_color",
        AtomState,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(723, "label_connector_width", AtomState, 2.0),
    s_float!(724, "label_connector_ext_length", AtomState, 2.5),
    s_color!(
        725,
        "label_bg_color",
        AtomState,
        patinae_color::ColorValue::INHERIT
    ),
    s_bool!(726, "use_geometry_shaders", Global, true),
    s_int!(727, "label_relative_mode", AtomState, 0, 0, 2),
    s_float3!(728, "label_screen_point", AtomState, 0.0, 0.0, 0.0),
    s_float!(729, "label_multiline_spacing", AtomState, 1.2),
    s_float!(730, "label_multiline_justification", AtomState, 1.0),
    s_float3!(731, "label_padding", AtomState, 0.2, 0.2, 0.0),
    s_float!(732, "label_bg_transparency", AtomState, 0.6),
    s_bool!(733, "label_bg_outline", AtomState, false),
    s_bool!(734, "ray_label_connector_flat", AtomState, true),
    s_float!(735, "dash_transparency", ObjectState, 0.0),
    s_int!(736, "pick_labels", ObjectState, 1),
    s_int!(737, "label_z_target", AtomState, 0),
    s_bool!(738, "session_embeds_data", Global, true),
    s_int!(739, "volume_mode", Global, 1),
    s_bool!(740, "trilines", Global, false),
    s_int!(741, "collada_export_lighting", Global, 0),
    s_int!(742, "collada_geometry_mode", Global, 1),
    s_bool!(743, "precomputed_lighting", Global, false),
    s_int!(744, "chromadepth", Global, 0),
    s_float!(745, "pse_export_version", Global, 0.0),
    s_bool!(746, "cif_use_auth", Global, true),
    s_string!(747, "assembly", Global),
    s_bool!(748, "cif_keepinmemory", Global, false),
    s_bool!(749, "pse_binary_dump", Global, false),
    // 750-797
    s_int!(750, "cartoon_gap_cutoff", ObjectState, 10),
    s_bool!(751, "ignore_case_chain", Global, false),
    s_float!(752, "valence_zero_scale", ObjectState, 0.2),
    s_int!(753, "valence_zero_mode", ObjectState, 1, 0, 2),
    s_int!(754, "auto_show_classified", Global, -1, -1, 3),
    s_bool!(755, "collada_background_box", Global, false),
    s_bool!(756, "pick32bit", Global, true),
    s_bool!(757, "cartoon_all_alt", ObjectState, false),
    s_int!(758, "display_scale_factor", Global, 1),
    s_bool!(759, "pick_shading", Global, false),
    s_string!(760, "fetch_type_default", Global),
    s_bool!(761, "editor_auto_measure", Global, true),
    s_bool!(762, "surface_smooth_edges", ObjectState, true),
    s_int!(763, "chem_comp_cartn_use", Global, 0),
    s_bool!(764, "colored_feedback", Global, false),
    s_bool!(765, "sdf_write_zero_order_bonds", Global, false),
    s_bool!(766, "cif_metalc_as_zero_order_bonds", Global, false),
    s_int!(767, "seq_view_gap_mode", Global, 1),
    s_int!(768, "internal_gui_name_color_mode", Global, 0, 0, 2),
    s_float!(769, "openvr_gui_fov", Global, 35.0, 0.0, 89.0),
    s_float!(770, "openvr_gui_alpha", Global, 1.0, 0.0, 1.0),
    s_int!(771, "openvr_gui_use_alpha", Global, 0, 0, 2),
    s_float!(772, "openvr_gui_scene_color", Global, 0.0),
    s_float!(773, "openvr_gui_scene_alpha", Global, 0.75),
    s_float!(774, "openvr_gui_back_color", Global, 0.2),
    s_float!(775, "openvr_gui_back_alpha", Global, 0.75),
    s_int!(776, "openvr_gui_use_backdrop", Global, 0, 0, 2),
    s_int!(777, "openvr_gui_overlay", Global, 0, 0, 2),
    s_int!(778, "openvr_gui_text", Global, 0),
    s_bool!(779, "openvr_disable_clipping", Global, false),
    s_float!(780, "openvr_near_plane", Global, 0.1),
    s_float!(781, "openvr_far_plane", Global, 100.0),
    s_bool!(782, "openvr_cut_laser", Global, false),
    s_float!(783, "openvr_laser_width", Global, 3.0),
    s_float!(784, "openvr_gui_distance", Global, 1.5),
    s_int!(785, "cartoon_smooth_cylinder_cycles", Global, 3),
    s_int!(786, "cartoon_smooth_cylinder_window", Global, 2),
    s_int!(787, "isosurface_algorithm", Global, 0, 0, 2),
    s_bool!(788, "cell_centered", Global, false),
    s_float!(789, "halogen_bond_distance", Global, 3.5),
    s_float!(790, "halogen_bond_as_donor_min_donor_angle", Global, 140.0),
    s_float!(
        791,
        "halogen_bond_as_donor_min_acceptor_angle",
        Global,
        90.0
    ),
    s_float!(
        792,
        "halogen_bond_as_acceptor_min_donor_angle",
        Global,
        120.0
    ),
    s_float!(
        793,
        "halogen_bond_as_acceptor_min_acceptor_angle",
        Global,
        90.0
    ),
    s_float!(
        794,
        "halogen_bond_as_acceptor_max_acceptor_angle",
        Global,
        170.0
    ),
    s_float!(795, "salt_bridge_distance", Global, 5.0),
    s_bool!(796, "use_tessellation_shaders", Global, true),
    s_color!(
        797,
        "cell_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    // 798-801: Silhouette settings
    s_bool!(798, "silhouettes", Global, false),
    s_float!(799, "silhouette_width", Global, 2.0, 0.5, 10.0),
    s_color!(
        800,
        "silhouette_color",
        ObjectState,
        patinae_color::ColorValue::INHERIT
    ),
    s_float!(801, "silhouette_depth_jump", Global, 0.03, 0.001, 0.5),
    // 802-806: Shading mode and Skripkin AO controls.
    // shading_mode: 0 = classic multi-light, 1 = skripkin ambient + AO
    s_int_enum!(
        802,
        "shading_mode",
        Global,
        0,
        0,
        2,
        &[
            ("classic", SettingValue::Int(0)),
            ("skripkin", SettingValue::Int(1)),
            ("full", SettingValue::Int(2))
        ]
    ),
    s_int!(803, "skripkin_directions", Global, 64),
    s_int!(804, "skripkin_map_size", Global, 128),
    s_float!(805, "skripkin_bias", Global, 0.025, 0.0, 0.1),
    s_float!(806, "skripkin_intensity", Global, 1.0, 0.0, 2.0),
    // 807: Surface individual chains
    s_bool!(807, "surface_individual_chains", Global, false),
    // 808: Transparent docked panels
    s_bool!(808, "transparent_panels", Global, false),
    // 809-811: Full shading mode (directional light shadows)
    s_int!(809, "shadow_map_size", Global, 1024),
    s_float!(810, "shadow_bias", Global, 0.01, 0.0, 0.1),
    s_float!(811, "shadow_intensity", Global, 0.5, 0.0, 2.0),
    // 812: Shadow PCF radius in texels.
    s_int!(812, "shadow_pcf", Global, 2),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setting_count() {
        assert_eq!(SETTINGS.len(), SETTING_COUNT);
    }

    #[test]
    fn test_get_setting() {
        let setting = get_setting(0).unwrap();
        assert_eq!(setting.name, "bonding_vdw_cutoff");
        assert_eq!(setting.id, 0);
    }

    #[test]
    fn test_get_setting_id() {
        let id = get_setting_id("sphere_quality").unwrap();
        assert_eq!(id, 87);
    }

    #[test]
    fn test_setting_indices_match() {
        for (idx, setting) in SETTINGS.iter().enumerate() {
            assert_eq!(
                setting.id as usize, idx,
                "Setting {} at index {} has mismatched id",
                setting.name, idx
            );
        }
    }

    #[test]
    fn test_all_settings_valid() {
        for setting in SETTINGS.iter() {
            // Verify type and default value match
            match (&setting.default, setting.setting_type) {
                (SettingValue::Bool(_), SettingType::Bool) => {}
                (SettingValue::Int(_), SettingType::Int) => {}
                (SettingValue::Float(_), SettingType::Float) => {}
                (SettingValue::Float3(_), SettingType::Float3) => {}
                (SettingValue::Color(_), SettingType::Color) => {}
                // String settings use Int(0) as placeholder
                (SettingValue::Int(0), SettingType::String) => {}
                // Blank settings use Int(0) as placeholder
                (SettingValue::Int(0), SettingType::Blank) => {}
                _ => panic!("Setting {} has mismatched type and default", setting.name),
            }
        }
    }
}
