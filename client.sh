#!/bin/bash

set -o errexit
set -o nounset

call() {
  local id=$1
  local method=$2
  local auth=$3
  shift 3
  local -a params
  params=( "$@" )

  # {"jsonrpc": "2.0", "method": "subtract", "params": [42, 23], "id": 1}
  local body
  body="$(
    printf '%s\n' "${params[@]}" | \
      jq --arg id "$id" --arg method "$method" \
        --null-input --compact-output '{
          "jsonrpc": "2.0",
          "id": $id,
          "method": $method,
          "params": [inputs]
        }'
    )"

  local request
  request=\
'POST / HTTP/1.1
Host: localhost'

  if [[ -n "$auth" ]]; then
    request="$request
X-Admin-Auth: $auth"
  fi

  request="$request"'
Content-Type: application/json
Content-Length: '"${#body}"'

'"$body"

  # printf '%s\n' "$request"
  printf '%s\n' "$request" | nc -C -N localhost 33481
}

echo "Calling f with no auth":
call 1 "f" "" 3 4
echo

echo "Calling f with invalid auth":
call 1 "f" "user" 3 4
echo

echo "Calling f with non-ASCII auth":
call 1 "f" $'non-\xf7ascii' 3 4
echo

echo "Calling f with proper auth":
call 1 "f" "root" 3 4
echo

echo "Calling g":
call 2 "g" "" 3 4
