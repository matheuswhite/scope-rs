import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import ansi_color_task

t = ansi_color_task(time_to_die=8)
t.start()

sleep(4)
keyboard.write("\x13")
t.join()

keyboard.press_and_release("esc")
