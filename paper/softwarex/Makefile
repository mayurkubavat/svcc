PDFLATEX ?= pdflatex
BIBTEX   ?= bibtex

.PHONY: all clean

all: main.pdf

main.pdf: main.tex refs.bib $(wildcard numbers.tex)
	$(PDFLATEX) -interaction=nonstopmode -halt-on-error main.tex
	$(BIBTEX) main
	$(PDFLATEX) -interaction=nonstopmode -halt-on-error main.tex
	$(PDFLATEX) -interaction=nonstopmode -halt-on-error main.tex

clean:
	rm -f main.aux main.bbl main.blg main.log main.out main.pdf main.spl
