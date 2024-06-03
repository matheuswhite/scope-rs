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
Set Framerate 24

Type@100ms "scope -c cmds.yaml serial COM1 115200"
Sleep 500ms
Enter

Type@300ms "/hello"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "/spaces"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "/scapes"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "/double_quotes"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "/single_quotes"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "/json"
Sleep 500ms
Enter
Sleep 2000ms

Type@300ms "/json_again"
Sleep 500ms
Enter
Sleep 2000ms

Escape
