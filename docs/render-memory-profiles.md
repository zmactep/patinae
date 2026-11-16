# Renderer Memory Profiles

Memory profiles control how much GPU memory Patinae keeps available for rendering,
which visual effects it permits, and how it handles scenes that exceed its
allocation budget. They affect the interactive viewport on desktop and web.

The default is automatic selection. You can override it at startup or change it
while working. A profile change rebuilds rendering resources; it does not edit
the molecular coordinates or your selection expressions.

## Choosing a profile

| Profile | When to use it | What to expect |
| --- | --- | --- |
| `performance` | Plenty of GPU memory, or you need the highest permitted shadow and ambient-occlusion quality | All viewport effects are available; more memory is retained for reuse. |
| `balanced` | You want to reduce memory use while keeping normal visual effects and interaction | Effects remain available, with smaller shadow/AO limits and less spare allocation. |
| `lite` | Limited GPU memory or a scene that puts the renderer under memory pressure | Fewer effects, simpler selection markers, smaller picking buffers, and tighter allocation limits. |
| `manual:<MiB>` | You want to choose the renderer's allocation-planning budget explicitly | The same visual restrictions as `lite`, with your chosen budget. |

`performance` is not a promise of higher frame rates. Results depend on the scene,
viewport size, GPU, and enabled effects. Likewise, increasing a manual budget
allows more allocations but does not restore effects disabled by `lite`.

## What changes in the viewport

The table lists permissions and upper limits. An effect marked “available” still
needs to be enabled in the scene settings. A profile does not turn every effect
on or force every resource to its maximum size.

| Feature | `performance` | `balanced` | `lite` / `manual` |
| --- | --- | --- | --- |
| Screen-space ambient occlusion (SSAO) | Available | Available | Disabled |
| FXAA edge smoothing | Available | Available | Disabled |
| Full selection and hover outlines | Available | Available | Selected-atom dots replace the full overlay; no full hover outline |
| Silhouette outlines | Available | Available | Disabled |
| Maximum directional shadow map | 4096 × 4096 | 2048 × 2048 | 1024 × 1024 |
| Maximum Skripkin AO tile size | 4096 × 4096 | 1024 × 1024 | 512 × 512 |
| Maximum Skripkin AO sampling directions | 256 | 64 | 16 |
| Picking-buffer width and height relative to viewport | 1/2 | 1/2 | 1/4 |
| Picking reprojection acceleration | Available | Available | Disabled |
| Shared allocation-planning budget | No profile-imposed cap | 2 GiB | 1 GiB for `lite`; chosen value for `manual` |

SSAO adds shading around nearby surfaces; FXAA smooths visible edges. With `lite`,
the image can look flatter and edges more jagged when those effects were enabled.
Smaller shadow maps can lose fine shadow detail. Skripkin AO remains available,
but its lower map and sampling limits can reduce shading detail.

Picking is the process of identifying an atom under the cursor. It remains
available in `lite` and `manual`, but resources are created on demand and only
when the allocation estimate fits the budget. Picking may be unavailable for a
large scene that fails that check. The smaller picking buffer does **not** lower
the displayed viewport resolution. Selection expressions and stored selections
remain usable even when the full visual overlay is disabled.

## How memory use changes

`performance` prepares SSAO and intermediate color textures eagerly and keeps
three desktop viewport handoff textures. `balanced` creates those optional
textures when needed and keeps two handoff textures. `lite` and `manual` keep
one. Handoff textures hold rendered frames for the desktop UI; fewer copies
reduce the memory cost of a large window.

Scene buffers also retain different amounts of spare capacity:

- `performance` grows buffers to the next power of two. It considers compaction
  after unused regions reach both 512 MiB and 50% of the logical allocation.
- `balanced` uses more moderate growth, with a 64 MiB spare-capacity cap. Its
  compaction thresholds are 128 MiB and 25%.
- `lite` grows close to the requested size, with a 16 MiB spare-capacity cap. Its
  compaction thresholds are 32 MiB and 10%.
- `manual` uses the same growth policy as `lite`. Its compaction byte threshold
  is 5% of the chosen budget, bounded to 8–32 MiB, together with the 10% threshold.

Compaction reclaims gaps left in rendering buffers. Keeping less spare capacity
saves memory, but subsequent scene changes may require more reallocation or
compaction work.

The budget is used to plan representation and picking allocations, accounting
for other tracked renderer resources. If a representation does not fit, Patinae
can reduce its quality, build it in chunks where supported, or skip it, and
reports a warning. Even `performance` must respect device resource limits.

These budgets are **not a hard limit on the application's total memory use**.
CPU-side molecules, the UI, plugins, driver allocations, and resources outside
the planner can add memory use. A `manual:1024` budget therefore does not mean
that the whole process will stay below 1 GiB. MiB and GiB are binary units:
1024 MiB = 1 GiB.

## When automatic selection chooses each profile

### Desktop: use the available native memory budget first

An explicit startup override takes priority. Otherwise, when the GPU backend
provides a usable native budget, Patinae computes:

```text
usable headroom = max(0, reported budget − current process GPU usage − 25% of reported budget)
```

The reserve leaves room for allocations outside the renderer's estimates.
Selection then follows these thresholds:

| Usable headroom after the reserve | Selected profile |
| --- | --- |
| At least 8 GiB | `performance` |
| At least 2 GiB, but less than 8 GiB | `balanced` |
| Less than 2 GiB | `lite` |

For example, a reported budget of 16 GiB with 4 GiB already used leaves 8 GiB
after the 4 GiB reserve, selecting `performance`. A budget of 8 GiB with 4 GiB
used leaves 2 GiB, selecting `balanced`.

The reported budget is an advisory amount available to this process, not simply
the GPU's advertised VRAM or the computer's installed RAM. Patinae can obtain it
from Metal on Apple platforms, DX12 on Windows, and Vulkan on Linux/Windows
when its memory-budget extension is available. Unified-memory systems can
include shared system memory in that budget.

The desktop makes a provisional choice before device creation and a final
choice from the created device, including allocations already made by the UI.
The final choice becomes the viewport's automatic baseline. These are startup
snapshots, not continuous monitoring of free memory.

### Without a native budget: use GPU capabilities

If the native budget is unavailable or zero, Patinae checks the exposed resource
limits in this order:

1. Select `lite` for a CPU/software or virtual GPU, a maximum single-buffer size
   below 1 GiB, a maximum storage-buffer binding below 512 MiB, or a maximum 2D
   texture dimension below 8192 pixels. Any one condition is enough.
2. Otherwise select `balanced` for the web viewer, a maximum single-buffer size
   below 4 GiB, or a maximum storage-buffer binding below 2 GiB minus one byte.
3. Otherwise select `performance`.

The browser viewer uses this capability path because it does not query native
process memory budgets. In automatic mode it therefore selects `lite` or
`balanced`, even on hardware that could run `performance` natively. These
resource limits describe what the backend exposes, not how much memory is free.
An integrated GPU is not automatically assigned `lite` just because it is
integrated. Selecting `lite` does not bypass hardware requirements: the desktop
app rejects CPU/software adapters, and every profile still requires a compatible
GPU device.

## Setting a profile

### Desktop startup

In a macOS/Linux shell:

```bash
PATINAE_RENDER_MEMORY_PROFILE=balanced patinae protein.pdb
PATINAE_RENDER_MEMORY_PROFILE=lite patinae protein.pdb
PATINAE_RENDER_MEMORY_PROFILE=performance patinae protein.pdb
PATINAE_RENDER_MEMORY_PROFILE=manual:1024 patinae protein.pdb
```

Leave `PATINAE_RENDER_MEMORY_PROFILE` unset for automatic startup selection.
The manual value must be a positive integer in MiB, without a unit suffix.

### During a session

Use the Patinae command line:

```text
set render_memory_profile, balanced
```

For a manual budget, set the amount first, then select `manual`:

```text
set render_memory_budget, 1024
set render_memory_profile, manual
```

Unlike the startup variable, the session setting uses two separate values.
`render_memory_budget` only applies to `manual`; a zero budget falls back to the
startup policy.

To return to the startup baseline:

```text
set render_memory_profile, auto
```

`auto` restores the policy chosen at startup, including any startup override.
It does not sample the operating system's memory budget again. Changing profiles
recreates the renderer on the existing GPU device, so the device's original
resource limits and allocation hints remain in effect. A switch may briefly
interrupt drawing while resources are rebuilt.

On macOS, starting in `balanced` or `lite` also disables the allocator's large
CPU heap cache unless `MallocLargeCache` was explicitly set. This can reduce
retained CPU memory. Switching profiles during a session cannot change that
startup allocator policy; Patinae reports when a restart is needed. `manual`
does not automatically apply this CPU allocator adjustment.

### Web viewer and benchmarks

For an embedded web viewer, set the `memoryProfile` constructor option or the
`memory-profile` custom-element attribute to `auto`, `performance`, `balanced`,
`lite`, or `manual:<MiB>`. See the [web viewer guide](../web/README.md) for the
embedding setup. The same session commands above can be sent through the web
viewer command interface.

The `render_loop` benchmarks accept startup overrides through
`BENCH_MEMORY_PROFILE`, using the same values as the desktop environment variable.

## Desktop recovery when GPU memory runs out

In the desktop app, a confirmed GPU out-of-memory error starts recovery:

1. Release temporary resources, compact scene buffers, and retry the current
   profile once.
2. If it still fails, rebuild with the next lower profile:
   `performance` → `balanced` → `lite`.
3. If `lite` still fails, stop retrying for the current scene state and report
   that rendering cannot continue. `manual` has no automatic lower profile;
   after its initial retry fails, it also stops.

Recovery can temporarily lower an explicitly requested profile. It does not
rewrite the requested setting. A scene change detected by the renderer resets
recovery to the baseline; choosing a different profile also resets recovery.
This is different from continuously upgrading profiles as free memory changes.

The web viewer supports profile selection and allocation-budget warnings, but
does not currently run this automatic out-of-memory downgrade sequence. Select
a lower profile explicitly if needed.

If warnings report skipped representations or unavailable picking, try hiding
unneeded representations, reducing their quality, or loading a smaller scene.
A smaller viewport can also reduce the memory used by screen-sized textures.

## Checking the active profile

Start the desktop app with its performance overlay enabled:

```bash
PATINAE_PERF=1 patinae protein.pdb
```

The overlay shows `PERF`, `BAL`, `LITE`, or `MAN` for the effective profile,
including a temporary recovery fallback. Its memory figures are estimates of
tracked renderer allocations, not the entire process's memory usage.

Startup logs report the selected profile and the selection reason:
`ExplicitOverride`, `NativeBudget`, or `CapabilityFallback`. Runtime warnings
explain disabled effects, reduced shadow/AO limits, representation budget
changes, and recovery transitions.

For implementation details, see the [profile policies](../crates/patinae-render/src/memory/policy.rs),
[automatic selector](../crates/patinae-render/src/memory/selection.rs), and
[recovery rules](../crates/patinae-render/src/memory/recovery.rs).
