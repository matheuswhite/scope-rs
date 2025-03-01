.ONESHELL:

VIDEO_DIRS := $(filter-out plugins/video.gif, $(filter-out __pycache__/video.gif, $(addsuffix video.gif, $(dir $(wildcard */)))))

.PHONY: all setup clean

all: setup ${VIDEO_DIRS}

setup:
	socat PTY,link=COM1,raw,echo=0 PTY,link=COM1_out,raw &

clean:
	rm -f ${VIDEO_DIRS}

%/video.gif: % %/video.py
	rm -rf $</video.cast
	. .venv/bin/activate; python $</video.py &
	asciinema rec -c "scope serial COM1 0" $</video.cast
	agg $</video.cast $</video.gif
