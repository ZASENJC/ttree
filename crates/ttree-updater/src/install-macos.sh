#!/bin/sh
set -eu
old_pid="$1"
destination="$2"
stage="$3"
backup="$stage/previous.app"
exec >"$stage/install.log" 2>&1
moved=0
installed=0
rollback() {
  result=$?
  if [ "$result" -ne 0 ]; then
    set +e
    if [ "$installed" = 1 ]; then mv "$destination" "$stage/failed.app"; fi
    if [ "$moved" = 1 ] && [ ! -e "$destination" ]; then mv "$backup" "$destination"; fi
    if ! kill -0 "$old_pid" 2>/dev/null; then
      /usr/bin/open "$destination" --args --show || true
    fi
    /usr/bin/open -a TextEdit "$stage/install.log" || true
  fi
}
trap rollback EXIT
# Never replace a live bundle. Abort if shutdown did not complete.
i=0
while kill -0 "$old_pid" 2>/dev/null; do
  i=$((i + 1))
  if [ "$i" -ge 120 ]; then echo "TTREE did not exit; update was cancelled."; exit 1; fi
  sleep 1
done
/bin/sh "$stage/verify.sh" "$stage/TTREE.app"
mv "$destination" "$backup"
moved=1
mv "$stage/TTREE.app" "$destination"
installed=1
/bin/sh "$stage/verify.sh" "$destination"
/usr/bin/open "$destination" --args --show
trap - EXIT
rm -rf "$stage"
