PAPER = main
export TEXINPUTS := ./template//:$(TEXINPUTS)
export BSTINPUTS := ./template//:$(BSTINPUTS)
export TFMFONTS := ./template//:$(TFMFONTS)
export T1FONTS := ./template//:$(T1FONTS)
export TEXFONTMAPS := ./template//:$(TEXFONTMAPS)

all: $(PAPER).pdf

$(PAPER).pdf: $(PAPER).tex numbers.tex refs.bib
	@test -f template/ieeeaccess.cls || { echo "template/ is empty: unpack the official IEEE Access LaTeX template there"; exit 1; }
	pdflatex -interaction=nonstopmode $(PAPER)
	bibtex $(PAPER)
	pdflatex -interaction=nonstopmode $(PAPER)
	pdflatex -interaction=nonstopmode $(PAPER)

clean:
	rm -f $(PAPER).aux $(PAPER).bbl $(PAPER).blg $(PAPER).log $(PAPER).out $(PAPER).pdf

.PHONY: all clean
