Output send_data.gif

Require echo

Set Padding 0
Set LineHeight 1.0
Set Shell "bash"
Set FontFamily "CaskaydiaMono Nerd Font"
Set WindowBar Colorful
Set BorderRadius 10
Set Theme "tokyonight"
Set FontSize 18
Set Width 1200
Set Height 750

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
