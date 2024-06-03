.ONESHELL:

VIDEO_DIRS := $(filter-out 000_scripts/video.gif, $(addsuffix video.gif, $(dir $(wildcard */))))

all: ${VIDEO_DIRS}

clean:
	rm -f ${VIDEO_DIRS}

%/video.gif: % %/video.tape
	@cd $<
	@./background.sh &
	@echo "$$!" > /tmp/$<.pid
	@vhs video.tape
	@cat /tmp/$<.pid | xargs pkill -P
	@rm -f *.txt

