# Current stable releases checked against upstream release APIs when bootstrapped.
# Update version and SHA-256 together; keep old result metadata for comparison.
# Archives are optional benchmark tools, not application dependencies.
REDIS_VERSION := 8.10.2
REDIS_SHA256 := 541a374b753a8405683dd88560465791acc98cf09445ef10ec1bd286904273e8
VALKEY_VERSION := 9.1.2
VALKEY_SHA256 := 19c23908e7d57e8d91ef85b41f5646307582f10f4f0fb999bbf89ed24ec9c983
# Optional same-setup CI comparison target, not an application dependency.
# BSL 1.1 permits non-production use; the workflow downloads the official
# x86_64 release and leaves the binary out of tracked source and artifacts.
DRAGONFLY_VERSION := 2.0.2
DRAGONFLY_SHA256 := eb9dfe654c69ab190cb3aa329ed3599ebe19c5e9a3a437bb7bc38ef52f665c88
DRAGONFLY_URL := https://github.com/dragonflydb/dragonfly/releases/download/v$(DRAGONFLY_VERSION)/dragonfly-x86_64.tar.gz
