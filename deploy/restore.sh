#!/bin/sh
# Restores a backup of the hub's data volume (docs/ops.md › Backup and
# restore). Run it in the directory of compose.yml:
#
#   ./restore.sh roedy-hub-20261127T030000Z.tar.gz    # replace the data with the archive's
#   ./restore.sh --recover                           # undo an interrupted restore
#
# It stops the hub and runs the steps in a helper container (alpine) with the
# volume: unpack the archive into the container, check its database with
# SQLite's integrity_check, copy it into the volume next to the current data,
# then swap by renaming – the current data is moved to .before-restore and
# only deleted once the new data is in place. Every failure puts the previous
# data back; the hub only starts again after a successful restore or recovery.
# secret.key in the volume is never touched.
#
# Environment: ROEDY_UID / ROEDY_GID (owner of the data, default 65532),
# ROEDY_HUB_VOLUME (default roedy_roedy-hub-data).
set -eu

volume=${ROEDY_HUB_VOLUME:-roedy_roedy-hub-data}
owner="${ROEDY_UID:-65532}:${ROEDY_GID:-65532}"
# The same alpine as the Dockerfile; plain tag, as in docs/ops.md.
alpine=alpine:3.24

die() {
  echo "restore: $*" >&2
  exit 1
}

# The steps inside the helper container: sh -e, $1 = restore | recover,
# $2 = archive path, D = data directory, R = staging directory, OWNER.
helper=$(
  cat <<'EOF'
D=${D:-/data}
R=${R:-/restore}
mode=$1
archive=${2:-}
# Moves the entries of $1 (filtered by the find tests after $2) into $2; fails
# if any move fails.
move_all() {
  src=$1
  dst=$2
  shift 2
  find "$src" -mindepth 1 -maxdepth 1 "$@" -exec sh -c 'dst=$1; shift; mv -f "$@" "$dst"/' sh "$dst" {} +
}
# Puts the previous data back. .before-restore/.complete marks that all of it
# was moved aside, so anything else in the volume is new data.
rollback() {
  if [ -e "$D/.before-restore/.complete" ]; then
    find "$D" -mindepth 1 -maxdepth 1 ! -name secret.key ! -name .before-restore ! -name .restore-new -exec rm -rf {} +
    rm -f "$D/.before-restore/.complete"
  fi
  move_all "$D/.before-restore" "$D"
  rmdir "$D/.before-restore"
  rm -rf "$D/.restore-new"
}

if [ "$mode" = recover ]; then
  if [ -d "$D/.before-restore" ]; then
    rollback
    echo "previous data put back"
  else
    rm -rf "$D/.restore-new"
    echo "nothing to recover"
  fi
  exit 0
fi

if [ -e "$D/.before-restore" ]; then
  echo "an interrupted restore left .before-restore: run ./restore.sh --recover first" >&2
  exit 1
fi
command -v sqlite3 >/dev/null || apk add --no-cache sqlite >/dev/null
rm -rf "$R"
mkdir -p "$R"
tar -xzf "$archive" -C "$R"
rm -f "$R/secret.key"
if [ "$(sqlite3 "$R/roedy.sqlite3" 'PRAGMA integrity_check' 2>&1)" != ok ]; then
  echo "the archive's database is missing or damaged; the data is unchanged" >&2
  exit 1
fi
rm -rf "$D/.restore-new"
if ! cp -a "$R" "$D/.restore-new"; then
  rm -rf "$D/.restore-new"
  echo "copying into the volume failed (disk full?); the data is unchanged" >&2
  exit 1
fi
mkdir "$D/.before-restore"
if ! move_all "$D" "$D/.before-restore" ! -name secret.key ! -name .before-restore ! -name .restore-new; then
  rollback
  echo "moving the current data aside failed; the data is unchanged" >&2
  exit 1
fi
touch "$D/.before-restore/.complete"
if ! move_all "$D/.restore-new" "$D"; then
  rollback
  echo "moving the archive's data in failed; the previous data is back" >&2
  exit 1
fi
rmdir "$D/.restore-new"
rm -rf "$D/.before-restore"
chown -R "$OWNER" "$D"
echo "restored"
EOF
)

case ${1:-} in
--recover)
  mode=recover
  dir=$PWD
  file=
  ;;
"" | -*)
  die "usage: ./restore.sh <archive.tar.gz> | --recover"
  ;;
*)
  mode=restore
  [ -f "$1" ] || die "$1 is not a file"
  dir=$(cd "$(dirname "$1")" && pwd)
  file=/backup/$(basename "$1")
  ;;
esac

docker compose stop roedy-hub
docker run --rm -e OWNER="$owner" -v "$volume:/data" -v "$dir:/backup:ro" "$alpine" \
  sh -ec "$helper" restore-helper "$mode" "$file" ||
  die "failed; the hub stays stopped (docs/ops.md › Backup and restore)"
docker compose start roedy-hub
