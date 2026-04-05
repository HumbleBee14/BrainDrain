#!/bin/sh
# Wrapper script for wal-g that loads credentials from envdir layout.
#
# PostgreSQL's archive_command and restore_command call this instead of
# wal-g directly. This avoids requiring the `envdir` binary (daemontools)
# in the postgres container image.
#
# Each file in /etc/wal-g/env/ is a variable (filename=name, content=value).
# This is the same envdir convention, implemented in pure shell.

WAL_G_DIR="/etc/wal-g/env"

if [ -d "$WAL_G_DIR" ]; then
    for f in "$WAL_G_DIR"/*; do
        [ -f "$f" ] || continue
        export "$(basename "$f")=$(cat "$f")"
    done
fi

exec wal-g "$@"
