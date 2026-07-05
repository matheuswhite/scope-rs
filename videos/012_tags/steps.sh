# Tags: reference values from the tag file with `@`. Tab autocompletes a tag
# name; the resolved value is sent when you press Enter.
type_str "@gr"
sleep 0.4
press Tab 0.6
sleep 0.3
tmux send-keys -t "$SESSION" Enter
sleep "$WAIT_END"
send_line "@reset"
send_line "@device_id"
sleep 1
press Escape
