# Send a File: stream a file's contents to the target with !send_file. The poll
# loop is slowed via opts.env so the transfer progress (10%, 20%, ...) is visible,
# and the far end is drained so the transfer doesn't stall on a full buffer.
drain_port
sleep 0.3
send_line "!send_file data.txt"
sleep 9
press Escape
