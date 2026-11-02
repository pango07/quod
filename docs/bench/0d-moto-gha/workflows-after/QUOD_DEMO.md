# Quod CI benchmark mirror of moto

This private repository mirrors consecutive first-parent commits of
[getmoto/moto](https://github.com/getmoto/moto) (Apache License 2.0; see `LICENSE` and upstream's notices). Every commit's
tree is upstream's at the commit named in its message
(`Upstream: getmoto/moto@<sha>`), with two changes, the same in every commit:

- `.github/workflows/` holds only this benchmark's workflows, and
  `.github/dependabot.yml` is removed (no upstream automation runs here);
- this file.

Upstream runs moto's unit tests as one job (`make test-only`). Here the same
tests run as one job per AWS service (`tests/test_<service>`), the shape a
monorepo's CI has, with upstream's pytest flags for each group, on Python
3.12, on self-hosted runners on one machine:

- `plain.yml`: the official GitHub Actions runner, label `plain`, job
  container by digest, a persistent pip cache;
- `quod.yml`: the official runner with Quod's container hooks, label `quod`,
  the same image, steps, pip cache location and machine;
- `hosted.yml`: the same jobs on GitHub's `ubuntu-latest`, for reference.

Nothing here is moto's own CI, and nothing is endorsed by moto's maintainers.
