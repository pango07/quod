cd /work/obsspeed/plain/cli
export PATH=/usr/local/go/bin:$PATH GOMODCACHE=/work/obsspeed/plain/gomod GOFLAGS="-mod=mod -buildvcs=false" GOPROXY=off CGO_ENABLED=0 GOCACHE=/tmp/gc-list HOME=/tmp
echo "module packages: $(go list ./... | wc -l)"
echo "module packages importing testing (transitively, non-test build): $(go list -f '{{.ImportPath}} {{join .Deps " "}}' ./... | grep -c ' testing\( \|$\)')"
echo "all packages (deps incl.) importing testing:"
go list -deps -f '{{.ImportPath}} {{join .Deps " "}}' ./... | grep ' testing\( \|$\)' | wc -l
echo "module packages importing testing directly:"
go list -f '{{.ImportPath}}: {{join .Imports " "}}' ./... | grep -E ' testing( |$)' | cut -d: -f1
