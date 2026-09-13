#!/bin/sh
# End-to-end demo of the EML-native Linux stack.
set -e
cd "$(dirname "$0")"
BIN="target/release"
[ -x "$BIN/emlinit" ] || BIN="target/debug"
R="/tmp/eml-linux-demo"
rm -rf "$R"; mkdir -p "$R/units" "$R/spool" "$R/root" "$R/pklog" "$R/mnt"

echo "== 1. services as .eml =="
cat > "$R/units/hello.eml" <<'U'
From: <hello@eml.local>
X-Entity-ID: hello
Content-Type: application/json

{"exec":["/bin/sh","-c","echo hi-from-emlinit"],"restart":"never"}
U
cat > "$R/units/tick.eml" <<'U'
From: <tick@eml.local>
X-Entity-ID: tick
Content-Type: application/json

{"exec":["/bin/sh","-c","echo tick"],"restart":"never","after":["hello"]}
U
"$BIN/emlinit" list --units "$R/units"

echo; echo "== 2. run + hash-chained log =="
"$BIN/emlinit" run --units "$R/units" --spool "$R/spool" --once
"$BIN/emllog" --spool "$R/spool" verify

echo; echo "== 3. package = .eml, install, rollback =="
"$BIN/emlpkg" build "$R/units" "$R/p.eml" --id units --version 1.0.0
"$BIN/emlpkg" verify "$R/p.eml"
"$BIN/emlpkg" install "$R/p.eml" --root "$R/root" --log "$R/pklog"
"$BIN/emlpkg" rollback --root "$R/root" --log "$R/pklog"

echo; echo "== 4. FUSE: mount the spool =="
"$BIN/emlfs" "$R/spool" "$R/mnt" &
FPID=$!; sleep 1
ls "$R/mnt" | head -3
d=$(ls "$R/mnt" | head -1)
echo "--- $d/payload.json ---"; cat "$R/mnt/$d/payload.json"
fusermount3 -u "$R/mnt" 2>/dev/null || umount "$R/mnt" 2>/dev/null
wait $FPID 2>/dev/null || true

echo; echo "== 5. fuzzy CA + game theory =="
"$BIN/emlca" diffuse --cells "255,0,0,0,0,0,0,0" --decay 1 --steps 4
"$BIN/emlca" vcg --bids "10,7,3,2" --slots 2
"$BIN/emlca" shapley --players "A,B,C" --v "A=1,B=2,C=0,AB=5,AC=4,BC=3,ABC=9"
"$BIN/emlca" nash --candidates "10,1;6,6;1,10" --d "0,0"

echo; echo "ALL OK"
