# Save history: while data streams in, hit Ctrl+S to save everything captured
# so far (the history box blinks and reports the saved file).
ansi_feed 16 &
feed_pid=$!
sleep 4
press C-s 1
wait "$feed_pid"
sleep 0.5
press Escape
