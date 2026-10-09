package main

import "ratingweb/ui"

// methodPage is the methodology: DESIGN.md sections 1–6 in the plain register
// of the site, with the PDF report as the formal source. The words are in the
// Catalog's method surface, written in English for the owner to translate.
func methodPage() (string, []ui.Item) {
	p := func(items ...ui.Item) ui.Node { return ui.Paragraph(ui.Inline(items...)) }
	t := ui.Text
	part := func(title string, ps ...ui.Item) ui.Node {
		return ui.Section(append([]ui.Item{ui.Subhead(t(title))}, ps...)...)
	}
	return S.Common.Nav.Method(), []ui.Item{
		ui.Pagehead(
			ui.Byline(ui.Inline(t(S.Method.Lede()+" "), link("/report.pdf", S.Method.Report()), t("."))),
		),
		part(S.Method.Team.Title(), p(t(S.Method.Team.Sum())), p(t(S.Method.Team.Value())), p(t(S.Method.Team.Strength()))),
		part(S.Method.Weight.Title(), p(t(S.Method.Weight.Product())), p(t(S.Method.Weight.Format())), p(t(S.Method.Weight.Field())), p(t(S.Method.Weight.Reach())), p(t(S.Method.Weight.Merged()))),
		part(S.Method.Time.Title(), p(t(S.Method.Time.Fade())), p(t(S.Method.Time.Roster())), p(t(S.Method.Time.Listed())), p(t(S.Method.Time.Search()))),
		part(S.Method.Player.Title(),
			p(t(S.Method.Player.ModelLead()+" "), ui.Em(t(S.Method.Player.Intensity())), t(S.Method.Player.ModelAlone()), ui.Sup(t(S.Method.Player.SupIntensity())),
				t(S.Method.Player.ModelTable()), ui.Sup(t(S.Method.Player.SupSum())), t(S.Method.Player.ModelTail())),
			p(t(S.Method.Player.Published())),
		),
		part(S.Method.Scale.Title(), p(t(S.Method.Scale.Anchor()+" "), link("/releases/", S.Method.Scale.Releases()), t(S.Method.Scale.Tail()))),
		part(S.Method.Uncertainty.Title(), p(t(S.Method.Uncertainty.Body()))),
		part(S.Method.Forecast.Title(), p(t(S.Method.Forecast.Lineup())), p(t(S.Method.Forecast.Editors())), p(t(S.Method.Forecast.Quality()))),
		part(S.Method.Difficulty.Title(), p(t(S.Method.Difficulty.Body()))),
		part(S.Method.Count.Title(), p(t(S.Method.Count.Formats())), p(t(S.Method.Count.Players()))),
		part(S.Method.Publication.Title(), p(t(S.Method.Publication.Body()))),
		part(S.Method.Reading.Title(), p(t(S.Method.Reading.Lead()+" "), link("/report.pdf", S.Method.Report()), t("."))),
	}
}
