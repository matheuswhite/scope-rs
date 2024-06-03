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

Type@300ms "!serial connect COM4 9600"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "!serial connect 9600"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "!serial connect COM4"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "!serial disconnect"
Sleep 500ms
Enter
Sleep 2000ms

Escape
