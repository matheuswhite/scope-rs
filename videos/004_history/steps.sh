# Written History: send a few messages, then walk back through them with Up.
send_line "Hello World"
send_line '$48,65,6c'
send_line "AT"
repeat_key Up 2
repeat_key Up 4
repeat_key Up 4
sleep 1
press Escape
