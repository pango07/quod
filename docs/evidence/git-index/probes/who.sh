set -eu
W=/work/tmp/gi-mech/cli
cd $W
mkdir -p /work/tmp/gi-wrap
real=$(command -v git)
cat > /work/tmp/gi-wrap/git <<EOW
#!/bin/sh
echo "\$PWD :: \$*" >> /work/tmp/gi-wrap/log
exec $real "\$@"
EOW
chmod +x /work/tmp/gi-wrap/git
rm -f /work/tmp/gi-wrap/log
PATH=/work/tmp/gi-wrap:$PATH go test -count=1 ./... > /work/tmp/gi-wrap/test.out 2>&1 || echo "go test exit $?"
echo "== git invocations whose cwd is inside the checkout:"
grep "^$W" /work/tmp/gi-wrap/log | sed "s#^$W#<repo>#" | sort | uniq -c | sort -rn | head -20
echo "== total invocations: $(wc -l < /work/tmp/gi-wrap/log)"
