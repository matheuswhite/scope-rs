import keyboard
import sys

sys.path.insert(0, "..")
from videos.lib import invisibles_task

t = invisibles_task(time_to_die=7)
t.start()
t.join()

keyboard.press_and_release("esc")
