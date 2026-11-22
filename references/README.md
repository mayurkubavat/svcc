# references/

The standing home for related work on svcc: SystemVerilog-to-SystemC and SystemC-to-RTL
translation, synthesizable subsets, HLS, hardware IRs, translation validation, equivalence
checking, and UVM-SystemC/CRAVE/FC4SC.

| File | Tracked in git | What it is |
| --- | --- | --- |
| `papers.toml` | yes | Single source of truth. One `[[paper]]` block per reference. |
| `INDEX.md` | yes (generated) | Readable index grouped by topic, with links and relevance notes. |
| `survey.bib` | yes (generated) | BibTeX for every entry not already in `paper/refs.bib`. |
| `pdfs/` | **no** (`.gitignore`) | Downloaded open-access PDFs, named `<key>.pdf`. |

## Everyday use

```sh
python3 scripts/refs.py fetch          # download every open-access PDF that is missing
python3 scripts/refs.py fetch wu2023sccl loow2025simsem   # only these keys
python3 scripts/refs.py all            # validate papers.toml, regenerate INDEX.md and survey.bib
```

The script uses only the Python standard library (3.11+).

Only entries with a `pdf_url` are downloaded, and a `pdf_url` is only ever a legal open copy:
arXiv, the DVCon proceedings archive, an open-access publisher PDF, or an institutional
repository. Paywalled papers are listed with their DOI link and marked *link only* in the index.
Entries with `oa_page` have a free copy that is not a direct PDF link (an author or repository
page); open it in a browser.

## Adding a paper

1. Append a block to `papers.toml`:

   ```toml
   [[paper]]
   key = "author2026word"          # BibTeX key: first author, year, one word
   section = "synth"               # one of the [[section]] ids at the top of the file
   priority = "core"               # core | related | background
   title = "..."
   authors = ["First Author", "Second Author"]
   year = 2026
   venue = "DAC 2026"
   type = "inproceedings"          # article | inproceedings | incollection | book | techreport | misc
   doi = "10.1145/..."             # optional
   arxiv = "2601.00001"            # optional
   url = "https://doi.org/10.1145/..."
   pdf_url = "https://arxiv.org/pdf/2601.00001"   # optional, legal open copy only
   note = "One or two sentences on why it matters to svcc."
   ```

2. Run `python3 scripts/refs.py all`, then `python3 scripts/refs.py fetch <key>`.
3. Commit `papers.toml`, `INDEX.md` and `survey.bib`. Never commit anything under `pdfs/`.

When a paper is cited in the manuscript, move its entry into `paper/refs.bib` under the same key
(expanding the abbreviated venue to the full proceedings name) and set `in_paper = true` here.
`survey.bib` leaves those entries out, so both files can be loaded together with
`\bibliography{refs,../references/survey}`.

## What the survey says about the synthesizable subset

The first priority for the tool is accurate conversion of synthesizable design models. The
entries marked **core** in `INDEX.md` are the ones that bear on that. In short:

- **There is no standard synthesizable SystemVerilog subset.** IEEE 1364.1-2002 covered Verilog
  and was withdrawn; `sutherland2014synth` is the de facto construct-by-construct list for
  SystemVerilog. On the SystemC side IEEE 1666.1-2016 (`ieee1666_1_2016`, free text in
  `accellera2016synth`) is normative. svcc's accepted subset should be stated against these two
  lists, construct by construct.
- **The semantics to preserve is now written down.** `loow2025simsem` and `chen2023essence` give
  formal semantics for synthesizable Verilog scheduling, and `mueller2001simsem` does the same
  for the SystemC kernel. `cummings2000nba` and `mills1999mismatch` catalog the blocking versus
  nonblocking and simulation-versus-synthesis traps; each is a ready-made negative test.
- **Closest tools to compare against.** SystemC to RTL: Intel's compiler (`moiseev2019icsc`) and
  SCCL (`wu2023sccl`). RTL to SystemC/C++: Verilator (`snyder2020verilator`), HIFSuite
  (`bombieri2010hifsuite`), v2c (`mukherjee2016v2c`) and RTL-to-C++ abstraction
  (`bombieri2013abstract`). `zheng2026bidir` is a 2026 bidirectional SystemC/Verilog translator
  and the nearest competing claim; it is paywalled and has not been read yet.
- **Checking each translation is well motivated.** `herklotz2021hlsfuzz` found miscompilations in
  every HLS tool tested, `kundu2010tvhls` applied translation validation to HLS, and
  `herklotz2021vericert` is the verified-compiler alternative. The C-versus-RTL equivalence line
  (`clarke2003consistency`, `koelbl2005tutorial`, `koelbl2009c2rtl`, `mathur2009functional`,
  `urard2008slec`) defines the notions of equivalence svcc's SEC implements.

None of these are cited in the manuscript's Related Work section yet except the entries marked
with a dagger in `INDEX.md`.
