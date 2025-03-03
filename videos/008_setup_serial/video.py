import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import write_input

write_input("!serial connect COM4 9600")
write_input("!serial connect 9600")
write_input("!serial connect COM4")
write_input("!serial disconnect")

sleep(1)
keyboard.press_and_release("esc")
