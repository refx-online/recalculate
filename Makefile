IMAGE := recalculate
VOLUME := meat-my-beat-i_data

.PHONY: build run

build:
	docker build -t $(IMAGE) .

run:
	docker run \
		--network=host \
		--env-file .env \
		-v $(VOLUME):/srv/root/.data \
		$(IMAGE) $(ARGS)
