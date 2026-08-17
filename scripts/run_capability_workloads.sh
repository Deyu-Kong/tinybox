#!/bin/bash
set -euo pipefail

echo "workload 1/3: offline static policy"
./scripts/test_c1.sh
echo "workload 2/3: install-to-build phase transition"
./scripts/test_c5.sh
echo "workload 3/3: adversarial Agent attempts"
./scripts/test_c6.sh
