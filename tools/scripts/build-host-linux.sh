#!/usr/bin/env bash
# Delegates to the canonical script inside the wandr-host submodule.
#
# This used to be a full copy, and the two had already drifted apart. Task 117
# added libvpx env plumbing that MUST stay in one place (the submodule owns
# scripts/libvpx-env.sh and its own CI), so this is now a one-line forwarder.
exec "$(cd "$(dirname "$0")/../.." && pwd)/runtime/wandr-host/scripts/build-host-linux.sh" "$@"
