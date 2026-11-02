#!/bin/bash
# Runs the per-process history measurement under the box's shared heavy lock.
cd /work/pm
exec >> /work/pm/launch.log 2>&1
echo "== start $(date -u +%FT%TZ) load $(cat /proc/loadavg)"
flock /work/heavy.lock timeout 9000 /usr/local/python/bin/python3 -I /work/pm/pm_history.py
echo "PM_EXIT=$? $(date -u +%FT%TZ)"
flock /work/heavy.lock timeout 1800 bash /work/pm/strace_survey.sh
echo "SURVEY_EXIT=$? $(date -u +%FT%TZ)"
echo LAUNCH_DONE
