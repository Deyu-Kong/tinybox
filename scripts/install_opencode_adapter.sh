#!/bin/sh
set -eu

workspace=${1:-.}
install_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -d "$install_root/adapters/opencode" ]; then
    tool_source=$install_root/adapters/opencode/bash.ts
    runtime_source=$install_root/adapters/shared/runtime.js
else
    tool_source=$install_root/share/tinybox/adapters/opencode/bash.ts
    runtime_source=$install_root/share/tinybox/adapters/shared/runtime.js
fi
target_dir=$workspace/.opencode/tools

mkdir -p "$target_dir"
for pair in "bash.ts:$tool_source" "runtime.js:$runtime_source"; do
    name=${pair%%:*}
    source_file=${pair#*:}
    target_file=$target_dir/$name
    if [ -e "$target_file" ] && ! cmp -s "$source_file" "$target_file"; then
        echo "refusing to overwrite existing $target_file" >&2
        exit 1
    fi
    cp "$source_file" "$target_file"
done
echo "$target_dir/bash.ts"
