//! CCP4/MRC electron density map reader
//!
//! Reads CCP4/MRC format map files (`.ccp4`, `.map`, `.mrc`) containing
//! 3D electron density data for crystallographic maps.

use std::io::Read;
use std::path::Path;

use crate::error::{IoError, IoResult};

/// Statistics from the CCP4 map header
#[derive(Debug, Clone)]
pub struct Ccp4Stats {
    /// Minimum density value
    pub dmin: f32,
    /// Maximum density value
    pub dmax: f32,
    /// Mean density value
    pub dmean: f32,
    /// RMS deviation of density
    pub rms: f32,
}

/// Parsed CCP4 map data
///
/// Contains the 3D density grid and associated metadata.
/// The grid is stored in XYZ order (X varies fastest).
#[derive(Debug, Clone)]
pub struct Ccp4Map {
    /// Grid dimensions (number of grid points along X, Y, Z)
    pub dims: [usize; 3],
    /// Cartesian origin in Angstroms
    pub origin: [f32; 3],
    /// Grid spacing per axis in Angstroms
    pub spacing: [f32; 3],
    /// Density values in XYZ order (X fastest)
    pub values: Vec<f32>,
    /// Unit cell parameters: a, b, c, alpha, beta, gamma
    pub cell: [f32; 6],
    /// Space group number
    pub space_group: u32,
    /// Density statistics
    pub stats: Ccp4Stats,
}

/// Read a CCP4 map file from a path
pub fn read_ccp4(path: &Path) -> IoResult<Ccp4Map> {
    let file = crate::compress::open_file(path)?;
    read_ccp4_from(file)
}

/// Read a CCP4 map from a reader
pub fn read_ccp4_from<R: Read>(mut reader: R) -> IoResult<Ccp4Map> {
    // Read header (256 words = 1024 bytes)
    let mut header = [0u8; 1024];
    reader
        .read_exact(&mut header)
        .map_err(|e| IoError::parse_msg(format!("Failed to read CCP4 header: {}", e)))?;

    // Detect endianness from machine stamp (bytes 212-215)
    // 0x44 0x41 = little-endian ("DA"), 0x11 0x11 = big-endian
    let little_endian = detect_endianness(&header)?;

    let read_i32 = |offset: usize| -> i32 {
        let bytes: [u8; 4] = header[offset..offset + 4].try_into().unwrap();
        if little_endian {
            i32::from_le_bytes(bytes)
        } else {
            i32::from_be_bytes(bytes)
        }
    };

    let read_f32 = |offset: usize| -> f32 {
        let bytes: [u8; 4] = header[offset..offset + 4].try_into().unwrap();
        if little_endian {
            f32::from_le_bytes(bytes)
        } else {
            f32::from_be_bytes(bytes)
        }
    };

    // Grid dimensions along columns, rows, sections
    let nc = read_i32(0) as usize;
    let nr = read_i32(4) as usize;
    let ns = read_i32(8) as usize;

    // Data mode (0=i8, 1=i16, 2=f32)
    let mode = read_i32(12);
    if mode != 0 && mode != 1 && mode != 2 {
        return Err(IoError::parse_msg(format!(
            "Unsupported CCP4 mode {}: only modes 0, 1, 2 are supported",
            mode
        )));
    }

    // Start indices
    let ncstart = read_i32(16);
    let nrstart = read_i32(20);
    let nsstart = read_i32(24);

    // Grid sampling along unit cell axes
    let nx = read_i32(28) as usize;
    let ny = read_i32(32) as usize;
    let nz = read_i32(36) as usize;

    // Cell dimensions
    let cell_a = read_f32(40);
    let cell_b = read_f32(44);
    let cell_c = read_f32(48);
    let cell_alpha = read_f32(52);
    let cell_beta = read_f32(56);
    let cell_gamma = read_f32(60);

    // Axis correspondence (1=X, 2=Y, 3=Z)
    let mapc = read_i32(64) as usize; // column axis
    let mapr = read_i32(68) as usize; // row axis
    let maps = read_i32(72) as usize; // section axis

    // Density statistics
    let dmin = read_f32(76);
    let dmax = read_f32(80);
    let dmean = read_f32(84);

    // Space group
    let ispg = read_i32(88) as u32;

    // Symmetry bytes
    let nsymbt = read_i32(92) as usize;

    // MRC2000 origin (words 50-52, byte offsets 196-207)
    // These take precedence over NCSTART/NRSTART/NSSTART when non-zero
    let origin_x = read_f32(196);
    let origin_y = read_f32(200);
    let origin_z = read_f32(204);

    // RMS deviation (word 55, byte offset 216)
    let rms = read_f32(216);

    // Validate
    if nc == 0 || nr == 0 || ns == 0 {
        return Err(IoError::parse_msg("CCP4 map has zero dimensions"));
    }
    if nx == 0 || ny == 0 || nz == 0 {
        return Err(IoError::parse_msg("CCP4 map has zero grid sampling"));
    }
    if !(1..=3).contains(&mapc) || !(1..=3).contains(&mapr) || !(1..=3).contains(&maps) {
        return Err(IoError::parse_msg(format!(
            "Invalid axis mapping: MAPC={}, MAPR={}, MAPS={}",
            mapc, mapr, maps
        )));
    }

    // Skip symmetry records
    if nsymbt > 0 {
        let mut sym_buf = vec![0u8; nsymbt];
        reader
            .read_exact(&mut sym_buf)
            .map_err(|e| IoError::parse_msg(format!("Failed to read symmetry records: {}", e)))?;
    }

    // Read density data
    let total_points = nc * nr * ns;
    let raw_values = read_density_data(&mut reader, total_points, mode, little_endian)?;

    // Compute per-axis spacing from cell dimensions and grid sampling
    // For orthogonal cells (alpha=beta=gamma=90), spacing = cell_dim / grid_sampling
    let spacing_cell = [cell_a / nx as f32, cell_b / ny as f32, cell_c / nz as f32];

    // Map axis indices: mapc/mapr/maps tell us which cell axis corresponds to col/row/section
    // mapc=1 means columns run along X, mapc=2 means columns run along Y, etc.
    // We need to reorder data from column/row/section order to XYZ order

    // Build the output grid dimensions and spacing in XYZ order
    let mut dims_xyz = [0usize; 3];
    let mut start_xyz = [0i32; 3];
    let dim_crs = [nc, nr, ns];
    let start_crs = [ncstart, nrstart, nsstart];

    dims_xyz[mapc - 1] = dim_crs[0]; // columns
    dims_xyz[mapr - 1] = dim_crs[1]; // rows
    dims_xyz[maps - 1] = dim_crs[2]; // sections

    start_xyz[mapc - 1] = start_crs[0];
    start_xyz[mapr - 1] = start_crs[1];
    start_xyz[maps - 1] = start_crs[2];

    let spacing_xyz = [
        spacing_cell[0], // X spacing
        spacing_cell[1], // Y spacing
        spacing_cell[2], // Z spacing
    ];

    // Compute origin in Cartesian coordinates
    // Prefer MRC2000 ORIGIN fields when non-zero (used by most modern software)
    let has_mrc_origin = origin_x != 0.0 || origin_y != 0.0 || origin_z != 0.0;
    let origin = if has_mrc_origin {
        [origin_x, origin_y, origin_z]
    } else {
        [
            start_xyz[0] as f32 * spacing_xyz[0],
            start_xyz[1] as f32 * spacing_xyz[1],
            start_xyz[2] as f32 * spacing_xyz[2],
        ]
    };

    // Reorder data from column/row/section to XYZ if needed
    let mut values = if mapc == 1 && mapr == 2 && maps == 3 {
        // Already in XYZ order (most common case)
        raw_values
    } else {
        reorder_to_xyz(&raw_values, &dim_crs, mapc, mapr, maps, &dims_xyz)
    };

    // Normalize values to sigma units: (value - mean) / rms
    // This makes contour level 1.0 = 1 sigma, matching standard crystallographic practice
    if rms > 1e-10 {
        let inv_rms = 1.0 / rms;
        for v in values.iter_mut() {
            *v = (*v - dmean) * inv_rms;
        }
    }

    // dims stores cell counts (vertex count = dims + 1 for Grid3D compatibility)
    // But for CCP4, dims ARE the number of grid points, and Grid3D.dims is number of cells.
    // Grid3D vertex_dims = dims + 1, so we need dims = grid_points - 1 for Grid3D.
    // However, CCP4 stores grid_points values, so we keep dims as grid_points - 1.
    let grid_dims = [dims_xyz[0] - 1, dims_xyz[1] - 1, dims_xyz[2] - 1];

    if cell_alpha != 90.0 || cell_beta != 90.0 || cell_gamma != 90.0 {
        eprintln!(
            "Warning: CCP4 map has non-orthogonal cell angles ({:.1}, {:.1}, {:.1}). \
             Map will be loaded assuming orthogonal axes.",
            cell_alpha, cell_beta, cell_gamma
        );
    }

    Ok(Ccp4Map {
        dims: grid_dims,
        origin,
        spacing: spacing_xyz,
        values,
        cell: [cell_a, cell_b, cell_c, cell_alpha, cell_beta, cell_gamma],
        space_group: ispg,
        stats: Ccp4Stats {
            dmin,
            dmax,
            dmean,
            rms,
        },
    })
}

/// Detect endianness from the machine stamp at bytes 212-215
fn detect_endianness(header: &[u8; 1024]) -> IoResult<bool> {
    // Check MAP magic at bytes 208-211
    let map_magic = &header[208..212];
    if map_magic != b"MAP " {
        return Err(IoError::parse_msg(format!(
            "Not a CCP4 map file: expected 'MAP ' at byte 208, got {:?}",
            std::str::from_utf8(map_magic).unwrap_or("<invalid>")
        )));
    }

    // Machine stamp: byte 212 indicates float format
    // 0x44 ('D') = IEEE little-endian (most common)
    // 0x11 = IEEE big-endian
    match header[212] {
        0x44 => Ok(true),  // little-endian
        0x11 => Ok(false), // big-endian
        _ => {
            // Fallback: try to detect from header values
            // If NC (word 1) as little-endian gives a reasonable value, use little-endian
            let nc_le = i32::from_le_bytes(header[0..4].try_into().unwrap());
            let nc_be = i32::from_be_bytes(header[0..4].try_into().unwrap());
            if nc_le > 0 && nc_le < 100_000 {
                Ok(true)
            } else if nc_be > 0 && nc_be < 100_000 {
                Ok(false)
            } else {
                Err(IoError::parse_msg("Cannot determine CCP4 file endianness"))
            }
        }
    }
}

/// Read density data based on mode
fn read_density_data<R: Read>(
    reader: &mut R,
    count: usize,
    mode: i32,
    little_endian: bool,
) -> IoResult<Vec<f32>> {
    match mode {
        0 => {
            // Mode 0: signed 8-bit integers
            let mut buf = vec![0u8; count];
            reader
                .read_exact(&mut buf)
                .map_err(|e| IoError::parse_msg(format!("Failed to read CCP4 data: {}", e)))?;
            Ok(buf.iter().map(|&b| b as i8 as f32).collect())
        }
        1 => {
            // Mode 1: signed 16-bit integers
            let mut buf = vec![0u8; count * 2];
            reader
                .read_exact(&mut buf)
                .map_err(|e| IoError::parse_msg(format!("Failed to read CCP4 data: {}", e)))?;
            Ok(buf
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&bytes| {
                    let val = if little_endian {
                        i16::from_le_bytes(bytes)
                    } else {
                        i16::from_be_bytes(bytes)
                    };
                    val as f32
                })
                .collect())
        }
        2 => {
            // Mode 2: 32-bit floats
            let mut buf = vec![0u8; count * 4];
            reader
                .read_exact(&mut buf)
                .map_err(|e| IoError::parse_msg(format!("Failed to read CCP4 data: {}", e)))?;
            Ok(buf
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&bytes| {
                    if little_endian {
                        f32::from_le_bytes(bytes)
                    } else {
                        f32::from_be_bytes(bytes)
                    }
                })
                .collect())
        }
        _ => Err(IoError::parse_msg(format!(
            "Unsupported CCP4 mode {}",
            mode
        ))),
    }
}

/// Reorder density data from column/row/section order to XYZ order
fn reorder_to_xyz(
    data: &[f32],
    dim_crs: &[usize; 3], // [nc, nr, ns]
    mapc: usize,
    mapr: usize,
    maps: usize,
    dims_xyz: &[usize; 3],
) -> Vec<f32> {
    let nc = dim_crs[0];
    let nr = dim_crs[1];
    let _ns = dim_crs[2];

    let mut result = vec![0.0f32; data.len()];

    // Iterate in CRS (file) order
    for s in 0..dim_crs[2] {
        for r in 0..dim_crs[1] {
            for c in 0..dim_crs[0] {
                let src_idx = c + r * nc + s * nc * nr;

                // Map CRS indices to XYZ indices
                let mut xyz = [0usize; 3];
                xyz[mapc - 1] = c;
                xyz[mapr - 1] = r;
                xyz[maps - 1] = s;

                let dst_idx = xyz[0] + xyz[1] * dims_xyz[0] + xyz[2] * dims_xyz[0] * dims_xyz[1];
                result[dst_idx] = data[src_idx];
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid CCP4 binary in memory for testing
    fn build_test_ccp4(dims: [usize; 3], values: &[f32]) -> Vec<u8> {
        let mut header = vec![0u8; 1024];
        let nc = dims[0];
        let nr = dims[1];
        let ns = dims[2];

        // Helper to write i32 at byte offset
        let write_i32 = |buf: &mut Vec<u8>, offset: usize, val: i32| {
            buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        };
        let write_f32 = |buf: &mut Vec<u8>, offset: usize, val: f32| {
            buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        };

        write_i32(&mut header, 0, nc as i32); // NC
        write_i32(&mut header, 4, nr as i32); // NR
        write_i32(&mut header, 8, ns as i32); // NS
        write_i32(&mut header, 12, 2); // MODE = float32

        // Start indices = 0
        write_i32(&mut header, 16, 0); // NCSTART
        write_i32(&mut header, 20, 0); // NRSTART
        write_i32(&mut header, 24, 0); // NSSTART

        // Grid sampling = dims (cell == grid extent)
        write_i32(&mut header, 28, nc as i32); // NX
        write_i32(&mut header, 32, nr as i32); // NY
        write_i32(&mut header, 36, ns as i32); // NZ

        // Cell dimensions: 10 Å per axis, 90° angles
        write_f32(&mut header, 40, nc as f32 * 1.0); // a
        write_f32(&mut header, 44, nr as f32 * 1.0); // b
        write_f32(&mut header, 48, ns as f32 * 1.0); // c
        write_f32(&mut header, 52, 90.0); // alpha
        write_f32(&mut header, 56, 90.0); // beta
        write_f32(&mut header, 60, 90.0); // gamma

        // Axis mapping: standard X, Y, Z
        write_i32(&mut header, 64, 1); // MAPC
        write_i32(&mut header, 68, 2); // MAPR
        write_i32(&mut header, 72, 3); // MAPS

        // Stats
        write_f32(&mut header, 76, -1.0); // DMIN
        write_f32(&mut header, 80, 1.0); // DMAX
        write_f32(&mut header, 84, 0.0); // DMEAN

        // Space group
        write_i32(&mut header, 88, 1); // P1

        // No symmetry records
        write_i32(&mut header, 92, 0); // NSYMBT

        // MAP magic
        header[208..212].copy_from_slice(b"MAP ");

        // Machine stamp: little-endian
        header[212] = 0x44;
        header[213] = 0x41;

        // RMS
        write_f32(&mut header, 216, 0.5);

        // Append density data
        let mut data = header;
        for &val in values {
            data.extend_from_slice(&val.to_le_bytes());
        }

        data
    }

    #[test]
    fn test_read_ccp4_basic() {
        let dims = [3, 3, 3];
        let total = 3 * 3 * 3;
        let values: Vec<f32> = (0..total).map(|i| i as f32 * 0.1).collect();

        let data = build_test_ccp4(dims, &values);
        let map = read_ccp4_from(std::io::Cursor::new(data)).unwrap();

        // dims in Grid3D convention = grid_points - 1
        assert_eq!(map.dims, [2, 2, 2]);
        assert_eq!(map.origin, [0.0, 0.0, 0.0]);
        assert_eq!(map.spacing, [1.0, 1.0, 1.0]);
        assert_eq!(map.values.len(), total);
        // Values are sigma-normalized: (raw - dmean) / rms = (raw - 0.0) / 0.5
        assert!((map.values[0] - 0.0).abs() < 1e-6);
        assert!((map.values[1] - 0.2).abs() < 1e-6); // 0.1 / 0.5 = 0.2
        assert_eq!(map.space_group, 1);
    }

    #[test]
    fn test_read_ccp4_stats() {
        let dims = [2, 2, 2];
        let values = vec![0.0f32; 8];
        let data = build_test_ccp4(dims, &values);
        let map = read_ccp4_from(std::io::Cursor::new(data)).unwrap();

        assert!((map.stats.dmin - (-1.0)).abs() < 1e-6);
        assert!((map.stats.dmax - 1.0).abs() < 1e-6);
        assert!((map.stats.dmean - 0.0).abs() < 1e-6);
        assert!((map.stats.rms - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_invalid_magic() {
        let mut data = build_test_ccp4([2, 2, 2], &[0.0; 8]);
        data[208..212].copy_from_slice(b"NOPE");
        assert!(read_ccp4_from(std::io::Cursor::new(data)).is_err());
    }

    #[test]
    fn test_mrc_origin() {
        let dims = [3, 3, 3];
        let total = 3 * 3 * 3;
        let values = vec![0.0f32; total];
        let mut data = build_test_ccp4(dims, &values);

        // Set MRC2000 ORIGIN fields (bytes 196-207)
        let write_f32 = |buf: &mut Vec<u8>, offset: usize, val: f32| {
            buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        };
        write_f32(&mut data, 196, 10.0); // ORIGIN X
        write_f32(&mut data, 200, 20.0); // ORIGIN Y
        write_f32(&mut data, 204, 30.0); // ORIGIN Z

        // Also set NCSTART to something non-zero to verify MRC origin takes precedence
        let write_i32 = |buf: &mut Vec<u8>, offset: usize, val: i32| {
            buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        };
        write_i32(&mut data, 16, 5); // NCSTART
        write_i32(&mut data, 20, 5); // NRSTART
        write_i32(&mut data, 24, 5); // NSSTART

        let map = read_ccp4_from(std::io::Cursor::new(data)).unwrap();

        // MRC origin should take precedence
        assert!((map.origin[0] - 10.0).abs() < 1e-6);
        assert!((map.origin[1] - 20.0).abs() < 1e-6);
        assert!((map.origin[2] - 30.0).abs() < 1e-6);
    }

    #[test]
    fn test_anisotropic_spacing() {
        let dims = [4, 6, 8];
        let total = 4 * 6 * 8;
        let values = vec![0.0f32; total];
        let mut data = build_test_ccp4(dims, &values);

        // Set cell dims to make spacing anisotropic: a=8, b=12, c=16
        // NX=4, NY=6, NZ=8 → spacing = [2.0, 2.0, 2.0]
        let write_f32 = |buf: &mut Vec<u8>, offset: usize, val: f32| {
            buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        };
        write_f32(&mut data, 40, 8.0); // a
        write_f32(&mut data, 44, 12.0); // b
        write_f32(&mut data, 48, 24.0); // c

        let map = read_ccp4_from(std::io::Cursor::new(data)).unwrap();
        assert!((map.spacing[0] - 2.0).abs() < 1e-6);
        assert!((map.spacing[1] - 2.0).abs() < 1e-6);
        assert!((map.spacing[2] - 3.0).abs() < 1e-6);
    }
}
