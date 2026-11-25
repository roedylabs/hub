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
# Everything that gets executed from the base (corepack, the pnpm shims and the
# pnpm versions in PNPM_HOME) must belong to this user: anything else was
# planted while the base was still open to others.
for _rm_dir in "$_rm_base/corepack" "$_rm_base/bin" "$PNPM_HOME"; do
  [ -e "$_rm_dir" ] || [ -L "$_rm_dir" ] || continue
  if ! _rm_foreign=$(find "$_rm_dir" ! -user "$(id -u)" -print) || [ -n "$_rm_foreign" ]; then
    echo "sandbox-env.sh: $_rm_dir contains files of other users; delete $_rm_base and try again" >&2
    unset _rm_base _rm_dir _rm_foreign
    return 1
  fi
done
unset _rm_dir _rm_foreign
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
