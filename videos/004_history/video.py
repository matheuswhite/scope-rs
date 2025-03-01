import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import write_input, repeat_key

write_input("Hello World")
write_input("$48,65,6c")
write_input("AT")

repeat_key("up", 2)
repeat_key("up", 4)
repeat_key("up", 4)

sleep(1)
keyboard.press_and_release("esc")
