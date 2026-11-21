PDFLATEX ?= /Library/TeX/texbin/pdflatex
BIBTEX   ?= /Library/TeX/texbin/bibtex

.PHONY: all clean arxiv-tar check

all: main.pdf

main.pdf: main.tex refs.bib
	$(PDFLATEX) -interaction=nonstopmode -halt-on-error main.tex
	$(BIBTEX) main
	$(PDFLATEX) -interaction=nonstopmode -halt-on-error main.tex
	$(PDFLATEX) -interaction=nonstopmode -halt-on-error main.tex

check: main.pdf
	@! grep -E "LaTeX Warning:|Package .* Warning:|Overfull \\\\hbox|Undefined" main.log || (echo "Found LaTeX warnings/overfull boxes in main.log" && exit 1)
	@! grep -E "Warning--" main.blg || (echo "Found BibTeX warnings in main.blg" && exit 1)
	@echo "LaTeX and BibTeX hygiene checks passed (0 warnings, 0 overfull boxes)."

arxiv-tar: main.pdf
	tar -czf arxiv-svcc-submission.tar.gz main.tex main.bbl refs.bib

clean:
	rm -f main.aux main.bbl main.blg main.log main.out main.pdf arxiv-svcc-submission.tar.gz
