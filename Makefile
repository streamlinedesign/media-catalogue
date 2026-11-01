# make          build ./katalog for this machine
# make dist     build release binaries for Linux, macOS and Windows into dist/
VERSION ?= $(shell git describe --tags --always 2>/dev/null || echo dev)
LDFLAGS := -s -w
PLATFORMS := linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64

katalog:
	CGO_ENABLED=0 go build -trimpath -ldflags "$(LDFLAGS)" -o katalog .

dist:
	@mkdir -p dist
	@for p in $(PLATFORMS); do \
	  os=$${p%/*}; arch=$${p#*/}; ext=; [ $$os = windows ] && ext=.exe; \
	  echo "building dist/katalog-$$os-$$arch$$ext"; \
	  CGO_ENABLED=0 GOOS=$$os GOARCH=$$arch go build -trimpath -ldflags "$(LDFLAGS)" -o dist/katalog-$$os-$$arch$$ext . || exit 1; \
	done

run: katalog
	./katalog serve

clean:
	rm -rf katalog dist site

.PHONY: katalog dist run clean
