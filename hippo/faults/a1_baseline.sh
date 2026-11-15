#!/usr/bin/env bash
# Baseline: a copy of the live store starts and answers.
. /tmp/libri-night/hippo/repro/env.sh
fresh base
t0=$(date +%s.%N); start base; t1=$(date +%s.%N)
stub base
echo "start: $(echo "$t1 - $t0" | bc) s"
hc base health
hc base status | head -20
hc base view | wc -c
time hc base post "Iris" <<<'{"kind":"user","text":"hello"}' | head -2
hc base 2>&1 | head -30
tail -5 $NIGHT/run/base/hippo.log | cut -c1-160
stop base
