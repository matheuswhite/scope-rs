import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import write_input

write_input("$48-65-6c-6c-6f")
write_input("$48,65,6c,6c,6f")
write_input("$48,65-6c,6c6f0a")

sleep(1)
keyboard.press_and_release("esc")
