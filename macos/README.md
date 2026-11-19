# macOS disk image

Install the packaging tool once with `brew install create-dmg`.
`make dmg` and `make dmg-full` package the signed/notarized app using the
same branded Finder layout. The release workflow installs the tool automatically.

To iterate on the artwork without rebuilding the application:

```sh
make dmg-package APP_DIR=/Applications/Patinae.app
```

The result is `target/Patinae.dmg`. This packaging-only target preserves the
provided app and its signing state; it does not sign or notarize it.
Use `DMG_PATH=target/Patinae-preview.dmg` to choose another output filename.

`dmg-background@2x.png` is the source artwork, generated for Patinae. Its
1536 × 1024 pixels map to a 768 × 512 point Finder content area. The packaging
script creates a TIFF with 1x and 2x representations for Retina displays.
The title, arrow, and installation instruction belong to the background;
the app and Applications icons are real Finder items, positioned by the script.

Packaging needs a macOS GUI session: create-dmg uses Finder to save the window
layout. Do not use its `--skip-jenkins` or `--sandbox-safe` options, which skip
the styling step. Open the final image in Finder to check the background,
icon positions, absence of scrollbars, and the Applications link after changes.
Eject the previous Patinae volume before checking a newly generated image.

## Retry notarization for a published release

In GitHub Actions, select **Re-notarize latest macOS release**, choose the
default branch and click **Run workflow**. The run resolves the published
GitHub release marked **Latest** at startup. Wait for the intended version to
appear as Latest before starting the workflow.

The workflow uses the signed `Patinae.app` in the published `Patinae.dmg`
and the release's existing Apple credentials. After Apple accepts the app,
the workflow staples the ticket and checks the app with Gatekeeper inside
the final compressed image. It then replaces the DMG at its existing download
URL. Apple submission reports are available in the run artifacts.

If publication is interrupted during the two asset renames, the original
image is available as `Patinae-original-<asset ID>.dmg`. If the release is
missing `Patinae.dmg`, rename that original back before retrying.
