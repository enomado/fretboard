# fretboard

Interactive fretboard visualizer built with [egui](https://github.com/emilk/egui). Shows notes, scales, and positions on a fretboard with logarithmic fret spacing.

<img width="1920" height="1993" alt="image" src="https://github.com/user-attachments/assets/cf6ab81d-5c72-4e94-8a23-64606bc1c60b" />

## Features


- Multiple tunings: cello (C-G-D-A), standard guitar (E), minor thirds
- Scales: major, minor, blues, dorian, phrygian, lydian, mixolydian, locrian
- Selectable root note
- Logarithmic fret spacing (matches real instrument geometry)
- Cello position brackets
- Hot-reload via [subsecond](https://github.com/jkelleyrtp/subsecond)

## Build & run

```sh
cargo run
```

## Android snail build

```sh
ANDROID_NDK_ROOT=/opt/android-sdk/android-ndk-r27c cargo apk build --lib
```

The debug APK is written to `target/debug/apk/fretboard.apk`.

## Screenshot

![screenshot](screenshot.png)
