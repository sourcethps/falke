# falke
A tool for movie making enhancements in THUG Pro.

## Features
- Enhances the Free Cam of THUG Pro (THUG2 Mod) to play interpolated Camera Paths between placeable snapshots (supports Transform, Rotation and FOV)

## Installation
I recommend using the "Ultimate ASI Loader" to inject the falke.dll into the THUG Pro Process.

### Using Ultimate ASI Loader
For convenience I've put the files directly inside the folder.
Place them next to your THUGPro.exe

If you want to do it yourself:
1. Download the 32Bit Version of "dinput8.dll" [Ultimate ASI Loader](https://github.com/ThirteenAG/Ultimate-ASI-Loader/releases).
2. Put the dinput8.dll next to your THUG Pro executable.
3. Put the "falke.asi" into a "plugins" folder, next to your THUG Pro Installation (f.e: C:\Users\YourUsername\AppData\Local\THUG Pro\plugins\falke.asi).

Additional Note: If you want to use Reshade you can just rename the Reshade DLL to "reshade.asi" and also put it there.

### Using a DLL Injector
Alternatively you can use any DLL Injector or probably the old "falke.exe" (if you still have an older release).

## How-To
The overlay should show once you move the camera in free cam mode.

You need at least two keyframes for a playable path. They are marked in the timeline and can be dragged.

Toggle the "Playback mode" to be able to enter freecam.

For playing the path hit the **SPACE** Key.

Press **F1** to toggle the UI. This is useful when recording. I advise to put the playtime marker a couple of seconds before the first keyframe, press play (Space) and toggle the UI (F1) when you want to record.

## FAQ
**Q: Is this compatible with THUG2?**

A: While this is possible, currently it is not compatible and support is not planned

## Known Issues
- Alt-Tabbing restores overlay window positions (won't fix for now)
- If the overlay doesn't show, try restarting the game
