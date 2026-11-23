# SoftwareX paper (svcc v0.1.2): reproducing the results

The paper describes release v0.1.2 (tag `v0.1.2`, Zenodo DOI
10.5281/zenodo.24455889, also on crates.io and PyPI).

```
git checkout v0.1.2
cargo build --release -p svcc
scripts/check.sh
pip install svcc==0.1.2 && svcc --help
```

The illustrative examples in the paper are the designs under `testdata/`;
quality numbers it quotes come from the same runs as `../ieee-access/README.md`
(repository proof, co-simulation corpus, SCAV comparison). Release checks:
journal entries `R0.1-release` and `R0.1.2-release-check`.
