# shellcheck shell=bash
# Environment for sandboxed shells where $HOME and /run are read-only (for
# example AI coding agents or locked-down CI runners).
# Usage: . scripts/dev/sandbox-env.sh
#
# Redirects caches (npm, pnpm, cargo), store locks, PNPM_HOME and corepack to
# a private directory in $TMPDIR and provides pnpm via corepack if it is missing (Node 26 no longer
# ships corepack).
# The pnpm version comes from "packageManager" in package.json: an installed
# pnpm 12 puts that version under $PNPM_HOME and starts it, without corepack.
# PNPM_HOME is set explicitly because the profile usually sets it to
# ~/.local/share/pnpm (then XDG_DATA_HOME does not apply).

_rm_base=${TMPDIR:-/tmp}/roedy-hub-dev
# The base must be this user's own directory that nobody else can write to:
# without TMPDIR it lies in the shared /tmp, where another user could have
# planted a corepack binary. Once checked, it is made private (700).
# umask 077 in a subshell: a new base is private whatever the caller's umask
# (mkdir -p -m only applies to the last directory, ShellCheck SC2174).
(umask 077 && mkdir -p "$_rm_base") || { unset _rm_base; return 1; }
# -perm -020/-002 instead of /022: BSD find (macOS) only knows the - and + forms.
_rm_loose=$(find "$_rm_base" -maxdepth 0 \( -perm -020 -o -perm -002 \) -print) ||
  { echo "sandbox-env.sh: cannot check the permissions of $_rm_base" >&2; unset _rm_base _rm_loose; return 1; }
if [ ! -O "$_rm_base" ] || [ -n "$_rm_loose" ]; then
  echo "sandbox-env.sh: $_rm_base must belong to $(id -un) and must not be writable by others" >&2
  unset _rm_base _rm_loose
  return 1
fi
unset _rm_loose
chmod 700 "$_rm_base"
export npm_config_cache="$_rm_base/npm-cache"
export COREPACK_HOME="$_rm_base/corepack-home"
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
export NODE_USE_ENV_PROXY=1 # otherwise Node fetch (corepack) does not use the sandbox proxy
export XDG_RUNTIME_DIR="$_rm_base/run" # pnpm 12 creates store locks there
export XDG_CACHE_HOME="$_rm_base/cache"
export XDG_DATA_HOME="$_rm_base/data"
export XDG_STATE_HOME="$_rm_base/state"
export PNPM_HOME="$_rm_base/pnpm-home" # version from "packageManager" (pnpm 12)
export CARGO_HOME="$_rm_base/cargo-home" # registry cache; ~/.cargo is not writable
# Nothing in the base may belong to another user: that was planted while the
# base was still open to others. Symlinks must resolve inside the private base,
# since a target elsewhere could be replaced through a directory others can
# write to. find does not follow links, so it visits every real directory of
# the base and every link in it, including links to directories. Dangling links
# are skipped (they run nothing; a target created later is caught next time);
# any other failure to resolve a link stops the script.
if ! command -v realpath >/dev/null 2>&1; then
  echo "sandbox-env.sh: realpath is required to check $_rm_base" >&2
  unset _rm_base
  return 1
fi
_rm_real=$(cd -P "$_rm_base" && pwd) || { unset _rm_base _rm_real; return 1; }
if ! _rm_foreign=$(find "$_rm_base" ! -user "$(id -u)" -print) || [ -n "$_rm_foreign" ] ||
  ! _rm_foreign=$(find "$_rm_base" -type l -exec sh -c '
    for link; do
      [ -e "$link" ] || continue
      target=$(realpath "$link") || exit 1
      case $target in "$0"/*) ;; *) echo "$link" ;; esac
    done' "$_rm_real" {} +) ||
  [ -n "$_rm_foreign" ]; then
  echo "sandbox-env.sh: $_rm_base contains files of other users or links out of it; delete it and try again" >&2
  unset _rm_base _rm_foreign _rm_real
  return 1
fi
unset _rm_foreign _rm_real
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$PNPM_HOME"
chmod 700 "$XDG_RUNTIME_DIR"

# A failed bootstrap returns non-zero, so that `. sandbox-env.sh && pnpm …`
# stops instead of going on without pnpm.
if ! command -v pnpm >/dev/null 2>&1; then
  if [ ! -x "$_rm_base/corepack/node_modules/.bin/corepack" ]; then
    npm install --silent --no-audit --no-fund --prefix "$_rm_base/corepack" corepack ||
      { echo "sandbox-env.sh: installing corepack failed" >&2; unset _rm_base; return 1; }
  fi
  mkdir -p "$_rm_base/bin"
  "$_rm_base/corepack/node_modules/.bin/corepack" enable --install-directory "$_rm_base/bin" pnpm ||
    { echo "sandbox-env.sh: corepack enable failed" >&2; unset _rm_base; return 1; }
  export PATH="$_rm_base/bin:$PATH"
fi
unset _rm_base
