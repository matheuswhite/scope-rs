import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import write_input, ansi_color_task, repeat_key

t = ansi_color_task(time_to_die=10)
t.start()

sleep(2)
keyboard.write("\x12")
sleep(4)
keyboard.write("\x12")
t.join()

keyboard.press_and_release("esc")
