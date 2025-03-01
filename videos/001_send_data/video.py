import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import write_input

write_input("HELLO")
write_input("Hello")
write_input("World")
write_input("AT")
write_input("OK")

sleep(1)
keyboard.press_and_release("esc")
