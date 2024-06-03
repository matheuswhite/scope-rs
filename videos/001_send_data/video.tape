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

Type@300ms "HELLO"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "Hello"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "World"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "AT"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "OK"
Sleep 500ms
Enter
Sleep 2000ms

Sleep 1s
Escape
