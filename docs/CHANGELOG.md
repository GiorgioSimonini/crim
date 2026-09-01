# Changelog

## 0.2.0

Renamed from `vidtrim` to **crim** (**cr**op & tr**im**).

### Added
- Application icon, embedded in the binary (`assets/icon.png`), plus a Linux
  desktop entry (`assets/crim.desktop`) and the matching app id so Wayland
  docks show it.
- **Audio / Muted** toggle in the toolbar: adds `-an` to the export *and* to
  the `ffplay` preview, and restarts a running preview so both stay in sync.
  Disabled when the source has no audio track.
- Dropdowns for the video and audio encoders and for the audio bitrate, with a
  `custom…` entry that reveals a text field for any other ffmpeg encoder.
- Stream copy handling: with `copy`, the encoder options (`-crf`, `-preset`,
  `-pix_fmt`, `-b:a`) are omitted, the corresponding controls are hidden, and a
  warning appears if a crop is active (a copy cannot apply a filter).
- `docs/` folder: `FEATURES.md` (current feature list), `tech_summary.md`,
  `CHANGELOG.md`. `INSTALL.md` became `README.md` and now covers usage.
- Two more tests (mute, stream copy): 8 in total.

### Changed
- The timeline now always grabs the **playhead** when it overlaps a trim
  cursor; trim points can still be set with **Set in** / **Set out**.
- Timeline strip grown from 62 px to 86 px, and the current-time label is kept
  fully inside the widget — it was clipped before.
- Toolbar status text (file, frame, crop, selection) reduced to 9.5 pt and
  reordered.
- Selection colour changed to the teal of the icon.

## 0.1.0

First release: trim with two timeline cursors, crop with handles on the
preview, live editable ffmpeg command, export with progress, playback with
audio through `ffplay`.
