# Record: Ctrl+R starts a record session (history box turns yellow); Ctrl+R
# again stops it. Only data captured between the two is recorded.
ansi_feed 20 &
feed_pid=$!
sleep 2
press C-r 1
sleep 4
press C-r 1
wait "$feed_pid"
sleep 0.5
press Escape
