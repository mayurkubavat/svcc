# IEEE Access submission: form fields

Copy-paste values for the IEEE Access submission site (ScholarOne Manuscripts), taken from
`main.tex` and `numbers.tex` on branch `ieee-proof`. Check each against the live form; field
names differ between the steps.

## Step: Type, Title and Abstract

- Manuscript type: Research Article
- Title: Translation Validation of SystemVerilog-to-SystemC RTL Conversion by Reading the Generated C++
- Running head: Kubavat: Translation Validation of SystemVerilog-to-SystemC RTL Conversion
- Abstract (241 words, plain text, numbers expanded from numbers.tex):

SystemC models of register-transfer level (RTL) designs are used for virtual platforms, software bring-up and simulation without a commercial license, and more of them now come from translators and language models. A wrong model often compiles cleanly: SystemVerilog evaluates an 8-bit sum in 8 bits, while C++ evaluates the same expression in 64, so a missing truncation keeps a carry that the RTL drops. Our first check of such translations re-read the generated C++ with SystemVerilog's rules and reported two wrong models as equivalent. This paper describes the check that replaced it in svcc, an open-source SystemVerilog-to-SystemC converter. The check parses the generated header with a model of C++17 and SystemC evaluation, turns undefined behavior into proof obligations, prints the result as SystemVerilog in which every width change is explicit, and proves it equivalent to a netlist of the source by induction over registers mapped by name. The reference netlist is built without any svcc code, but both sides use the same SystemVerilog front end. On 50 designs the check proves 45 and reports 5 as outside its model. In a mutation experiment it accepted none of the 17 mutants that compiled SystemC showed to be wrong. On 2,184 modules from 16 open-source code bases, 1,185 conversions match Verilator in random co-simulation, and the check proves 600 of them equivalent. The runs also found defects in the front end, in svcc and in Verilator. SystemC process scheduling is checked by co-simulation.

## Step: Keywords

Equivalence checking; hardware description languages; RTL; SystemC; SystemVerilog; translation validation

(The site asks for keywords from the IEEE taxonomy list; nearest terms: Formal verification,
Hardware design languages, Integrated circuit modeling, Simulation, Software tools.)

## Step: Authors and Institutions

- Mayur Kubavat, sole and corresponding author
- E-mail: mkubavat01@gmail.com
- ORCID: 0009-0009-1087-8558
- Institution: Independent Researcher
- City / State / Postal code / Country: Mountain View, CA 94040, USA
- Footnote shown in the paper: done independently, on personal equipment and in a personal
  capacity; not affiliated with or endorsed by any employer.

## Step: Details and Comments

- Funding: none (stated in the Acknowledgment)
- Conflicts of interest: none (stated in the Acknowledgment)
- Previously published / under review elsewhere: No. A preprint of a version of this work is
  posted (or will be posted) on arXiv; the companion software paper "svcc: SystemVerilog-to-SystemC
  Conversion with a Formal Check of the Generated C++" is submitted to SoftwareX and does not
  repeat this evaluation.
- Data and code availability: https://github.com/mayurkubavat/svcc, archived on Zenodo,
  DOI 10.5281/zenodo.24455890 (v0.1.1).
- Generative AI use: as stated in the paper's Acknowledgment.
- Suggested reviewers: optional; none prepared.

## Step: File Upload

- Main document: PDF built from `main.tex` with the official IEEE Access template in `template/`.
- Source files: main.tex, numbers.tex, refs.bib, fig-gate.pdf (the site asks for LaTeX source
  after acceptance or at upload, depending on the step).
- Cover letter: not yet written for IEEE Access (the SoftwareX one is in
  `paper/softwarex/cover-letter.txt` and can be adapted).

## Before pressing Submit

- Build main.pdf with the official template (draft mode is already off) and upload main.tex,
  numbers.tex, refs.bib, fig-gate.pdf and photo-kubavat.jpg with it.
- The final Submit, the copyright form and any payment are the author's.
