import keyboard
from time import sleep
import sys

sys.path.insert(0, "..")
from videos.lib import write_input, kill_socat, spawn_socat

kill_socat()
sleep(1)
write_input("Hello")
sleep(2)

spawn_socat()
sleep(1)
write_input("Hello")
sleep(1)
write_input("hello")

sleep(1)
keyboard.press_and_release("esc")
