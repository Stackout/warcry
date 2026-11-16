#!/bin/sh
set -e
cd "$(dirname "$0")"
for file in ../WarcryBeacon/*.lua; do
  luac -p "$file"
done
lua ledger_test.lua
