Output video.gif

Require echo

Set Padding 2
Set LineHeight 1.0
Set Shell "bash"
Set FontFamily "CaskaydiaMono Nerd Font"
Set WindowBar Colorful
Set BorderRadius 10
Set Theme "tokyonight"
Set FontSize 14
Set Width 902
Set Height 549
Set Framerate 24

Type@100ms "scope serial COM1 115200"
Sleep 500ms
Enter

Type@200ms "$48 65 6c 6c 6f"
Sleep 500ms
Enter
Sleep 2000ms

Type@200ms "$48,65,6c,6c,6f"
Sleep 500ms
Enter
Sleep 2000ms

Type@200ms "$48,65 6c,6c6f0a"
Sleep 500ms
Enter
Sleep 2000ms

Escape
