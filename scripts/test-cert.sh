#!/bin/sh
# A self-signed certificate for tests of the built-in TLS with ROEDY_TLS=files
# (scripts/image-smoke.sh, scripts/compose-smoke.sh, deploy/compose.test.yml):
# ECDSA P-256, valid for 30 days from now, the given host names as DNS names.
# Writes <dir>/cert.pem and <dir>/key.pem; clients trust cert.pem itself (e.g.
# `curl --cacert <dir>/cert.pem`). Never for production.
#
#   scripts/test-cert.sh <dir> <host> [<host> …]
#
# The key is readable by everyone (0644): the hub runs as another user
# (65532) in its container and reads it from a read-only bind mount. That is
# fine for a throwaway test key and nothing else. Needs openssl.
set -eu

dir=${1:?usage: scripts/test-cert.sh <dir> <host> [<host> …]}
shift
[ "$#" -gt 0 ] || {
  echo "usage: scripts/test-cert.sh <dir> <host> [<host> …]" >&2
  exit 2
}

san=
for host in "$@"; do
  san="${san:+$san,}DNS:$host"
done

mkdir -p "$dir"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -days 30 -subj "/CN=$1" -addext "subjectAltName=$san" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=serverAuth" \
  -keyout "$dir/key.pem" -out "$dir/cert.pem" 2>/dev/null
chmod 0755 "$dir"
chmod 0644 "$dir/cert.pem" "$dir/key.pem"
