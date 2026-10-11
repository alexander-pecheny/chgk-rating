package main

import (
	"errors"
	"fmt"
	"math"

	"ratingweb/ui"
)

// The listings: the two ranking tables, the tournaments, the releases and the
// anomaly scores. Each is a page of pageSize rows with a pager over and under.

// paging is one page of a listing: which, of how many, the rank it starts
// after, and the addresses of its pages.
type paging struct {
	page, pages, lo int
	first, prefix   string
}

func paginate(n, page int, first, prefix string) paging {
	pages := max(1, (n+pageSize-1)/pageSize)
	page = min(max(page, 1), pages)
	return paging{page, pages, (page - 1) * pageSize, first, prefix}
}

func (p paging) hi() int { return p.lo + pageSize }

func (p paging) nav() ui.Node {
	return ui.Pager(ui.Cur(itoa(int64(p.page))), ui.Total(itoa(int64(p.pages))), ui.First(p.first), ui.Prefix(p.prefix))
}

// title is the listing's name, with the page number past the first.
func (p paging) title(name string) string {
	if p.page > 1 {
		return S.Common.PageTitle(name, itoa(int64(p.page)))
	}
	return name
}

// framed is the listing's table between two pagers.
func (p paging) framed(t ui.Node) []ui.Item { return []ui.Item{p.nav(), t, p.nav()} }

// rankLine is a row of a ranking table: its rank, whether the rating is
// provisional, and the cells after the rank.
type rankLine struct {
	rank  int64
	prov  bool
	cells []ui.Item
	// ranked is a provisional line that keeps its place in a single ranking.
	ranked bool
}

// rankRows are the table's rows, with the band before the first provisional
// one unless the page starts past it.
func rankRows(lines []rankLine, split bool, splitText string, cols int) []ui.Item {
	var out []ui.Item
	for _, l := range lines {
		if l.prov && !l.ranked && !split {
			split = true
			band := textCell(splitText, ui.Colspan(itoa(int64(cols))))
			out = append(out, ui.Trow(ui.SplitRow(), ui.ProvRow(), band))
		}
		out = append(out, rankRow(l))
	}
	return out
}

// rankRow is a provisional rating without a rank, or a ranked one.
func rankRow(l rankLine) ui.Node {
	rank := itoa(l.rank)
	unranked := l.prov && !l.ranked
	if unranked {
		rank = dash
	}
	tr := append([]ui.Item{numCell(rank)}, l.cells...)
	if unranked {
		tr = append(tr, ui.ProvRow())
	}
	return ui.Trow(tr...)
}

func hiddenOf(lines []rankLine) int {
	n := 0
	for _, l := range lines {
		if l.prov {
			n++
		}
	}
	return n
}

// rankTable is a ranking table under its header, split before the first
// provisional rating.
func rankTable(header []ui.Item, lines []rankLine, split bool, splitText string) ui.Node {
	return table(header, rankRows(lines, split, splitText, len(header))...)
}

func (s *site) teamTable(page int) (string, []ui.Item, error) {
	p := paginate(s.nTeams, page, "/", "/page/")
	firm, err := s.q.firmTeams()
	rows, err2 := s.q.teamRanks(p.lo, p.hi(), false)
	if err = errors.Join(err, err2); err != nil {
		return "", nil, err
	}
	lines := make([]rankLine, len(rows))
	for i, r := range rows {
		lines[i] = s.teamLine(r, nil, false)
	}
	var sec []ui.Item
	// The switch only matters when some team is provisional.
	if firm < s.nTeams {
		sec = append(sec, provToggle(hiddenOf(lines), firm/pageSize+1, p.page))
	}
	sec = append(sec, p.framed(rankTable(s.teamCols(s.q.trb, false), lines, p.lo >= firm, S.Teams.ProvSplit()))...)
	lede := head(append([]ui.Item{ui.Text(S.Teams.Lede(s.releaseDate, s.nTeams))}, s.orderSwitch(false)...)...)
	return p.title(S.Common.Nav.Teams()), []ui.Item{lede, ui.Section(sec...)}, nil
}

// trbTable is the team table at /trb/, ordered by ТРБ, the strength of each
// team's base roster: one ranking, provisional ratings in their places.
func (s *site) trbTable(page int) (string, []ui.Item, error) {
	n, err := s.q.trbTeams()
	p := paginate(n, page, "/trb/", "/trb/page/")
	rows, err2 := s.q.teamRanks(p.lo, p.hi(), true)
	if err = errors.Join(err, err2); err != nil {
		return "", nil, err
	}
	lines := make([]rankLine, len(rows))
	for i, r := range rows {
		lines[i] = s.teamLine(r, nil, true)
		lines[i].rank, lines[i].ranked = intOr(r.trbRank), true
	}
	sec := p.framed(rankTable(s.teamCols(true, true), lines, true, ""))
	lede := head(append([]ui.Item{ui.Text(S.Teams.LedeTrb(s.releaseDate, n))}, s.orderSwitch(true)...)...)
	return p.title(S.Common.Nav.Teams()), []ui.Item{lede, ui.Section(sec...)}, nil
}

// orderSwitch says, after the lede, which order the team table is in and
// links to the other, for a phone, whose cards have no headers to click.
func (s *site) orderSwitch(byTRB bool) []ui.Item {
	if !s.q.trb {
		return nil
	}
	rating, trb := link("/", S.Teams.ByRating()), link("/trb/", S.Teams.ByTrb())
	if byTRB {
		trb = ui.Text(S.Teams.ByTrb())
	} else {
		rating = ui.Text(S.Teams.ByRating())
	}
	return []ui.Item{ui.Text(" " + S.Teams.Order() + " "), rating, ui.Text(" · "), trb, ui.Text(".")}
}

func intOr(v *int64) int64 {
	if v == nil {
		return 0
	}
	return *v
}

// teamCols are the team table's header; the published engine has no ± for a
// team and shows the results its rating rests on instead. With trb the table
// has a ТРБ column, and the rating's and the ТРБ's headers link to the table
// ordered by each; byTRB says which order this one is in.
func (s *site) teamCols(trb, byTRB bool) []ui.Item {
	spread := numCol("±", S.Common.Col.SigmaTitle())
	if s.q.s2 {
		spread = numCol(S.Teams.Col.Results(), S.Teams.Col.ResultsTitle())
	}
	rating := numCol(S.Common.Col.Rating())
	cols := []ui.Item{numCol(S.Common.Col.Place()), nameCol(S.Common.Col.Team()), rating}
	if trb {
		cols[2] = sortCol(S.Common.Col.Rating(), "", "/", !byTRB)
		cols = append(cols, sortCol(S.Teams.Col.Trb(), S.Teams.Col.TrbTitle(), "/trb/", byTRB))
	}
	return append(cols, spread, numCol(S.Common.Col.Change(), S.Common.Col.ChangeTeamTitle()))
}

// sortCol is the header of a column the table can be ordered by: a link to
// that order, or, when the table is in it, the plain heading.
func sortCol(text, title, href string, current bool) ui.Node {
	props := []ui.Item{ui.Num()}
	if title != "" {
		props = append(props, ui.Title(title))
	}
	if current {
		return ui.Hcell(append([]ui.Item{ui.Text(text + " ↓")}, props...)...)
	}
	if title == "" {
		props = append(props, ui.Title(S.Teams.Col.SortTitle()))
	}
	return ui.Hcell(append([]ui.Item{ui.Inline(link(href, text))}, props...)...)
}

// teamLine and playerLine link a name to the page as of the release the
// table is from; nil is today's.
func (s *site) teamLine(r teamRank, at *release, byTRB bool) rankLine {
	spread := numCell(disp(num(r.sigma)), ui.Faint())
	if s.q.s2 {
		spread = numCell(fmt.Sprintf("%.0f", num(r.evi)), ui.Faint())
	}
	name := nameCell(asOfHref(teamHref(r.id), at), teamName(r.name, r.id), provMark(r.prov), town(str(r.town)))
	cells := []ui.Item{name, numCell(disp(num(r.display)), ui.Bold())}
	if at == nil && s.q.trb {
		cells = append(cells, numCell(dispOpt(r.trb)))
	}
	if byTRB {
		// The number a phone's card shows is the bold one: the order's.
		cells[1], cells[2] = numCell(disp(num(r.display))), numCell(dispOpt(r.trb), ui.Bold())
	}
	return rankLine{rank: r.rank, prov: r.prov, cells: append(cells, spread, changeCell(r.change))}
}

func (s *site) playerTable(page int) (string, []ui.Item, error) {
	p := paginate(s.nPlayers, page, "/players/", "/players/page/")
	firm, err := s.q.firmPlayers()
	rows, err2 := s.q.playerRanks(p.lo, p.hi())
	if err = errors.Join(err, err2); err != nil {
		return "", nil, err
	}
	single := s.q.sc.rankSigmas > 0
	lines := make([]rankLine, len(rows))
	for i, r := range rows {
		lines[i] = s.playerLine(r, nil)
		lines[i].ranked = single
	}
	sec := p.framed(rankTable(playerCols(s.q.sc.rankSigmas), lines, p.lo >= firm, S.Players.ProvSplit()))
	if !single {
		firstProv := 0
		if firm < s.nPlayers {
			firstProv = firm/pageSize + 1
		}
		sec = append([]ui.Item{provToggle(hiddenOf(lines), firstProv, p.page)}, sec...)
	}
	lede := head(ui.Text(S.Players.Lede(s.releaseDate, s.nPlayers)))
	return p.title(S.Common.Nav.Players()), []ui.Item{lede, ui.Section(sec...)}, nil
}

func playerCols(rankSigmas float64) []ui.Item {
	place := numCol(S.Common.Col.Place())
	if rankSigmas > 0 {
		place = numCol(S.Common.Col.Place(), S.Players.Col.PlaceTitle(plainNum(rankSigmas)))
	}
	k := plainNum(rankSigmas)
	return []ui.Item{place, nameCol(S.Common.Col.Player()), numCol(S.Common.Col.Rating(), S.Players.Col.RatingTitle(k)),
		numCol("R", S.Players.Col.RTitle()), numCol("±", S.Common.Col.SigmaTitle()),
		numCol("θ", S.Players.Col.ThetaTitle()), numCol("σ", S.Players.Col.SigmaTitle()), numCol("v", S.Players.Col.VolTitle()),
		numCol(S.Players.Col.GamesYear()), numCol(S.Common.Col.Change(), S.Common.Col.ChangePlayerTitle())}
}

// playerLine shows the rating, R_pub, which orders the table, beside the
// estimate R it is the lower bound of, and the model's own numbers.
func (s *site) playerLine(r playerRank, at *release) rankLine {
	return rankLine{rank: r.rank, prov: r.prov, cells: []ui.Item{
		nameCell(asOfHref(playerHref(r.id), at), playerName(r.name, r.id), provMark(r.prov)),
		numCell(disp(s.q.sc.floor(num(r.display), num(r.sigma))), ui.Bold()),
		numCell(disp(num(r.display))),
		numCell(disp(num(r.sigma)), ui.Faint()),
		modelCell(r.theta), modelCell(r.sigmaT), modelCell(r.vol),
		numCell(itoa(count(r.games))),
		changeCell(r.change),
	}}
}

// modelCell is one of the model's numbers, to the digits /method/ uses.
func modelCell(v *float64) ui.Node {
	if v == nil {
		return dashCell()
	}
	return numCell(model(*v), ui.Faint())
}

// The digits the tournament table prints its metrics to.
const (
	weightDigits = 2
	llDigits     = 3
)

func (s *site) tournamentTable(page int) (string, []ui.Item, error) {
	n, err := s.q.tournamentCount()
	p := paginate(n, page, "/tournaments/", "/tournaments/page/")
	rows, err2 := s.q.tournaments(pageSize, p.lo)
	if err = errors.Join(err, err2); err != nil {
		return "", nil, err
	}
	trs := make([]ui.Item, len(rows))
	for i, r := range rows {
		trs[i] = tournamentRow(r)
	}
	t := table(tournamentCols(), trs...)
	return p.title(S.Common.Nav.Tournaments()), []ui.Item{head(ui.Text(S.Tournaments.Lede(n))), ui.Section(p.framed(t)...)}, nil
}

func tournamentCols() []ui.Item {
	return []ui.Item{
		hcell(S.Common.Col.Date()), nameCol(S.Common.Col.Tournament()), hcell(S.Common.Col.Type()),
		numCol(S.Tournaments.Col.Weight()), numCol(S.Tournaments.Col.Teams()), numCol(S.Tournaments.Col.Questions()),
		numCol(S.Common.Col.Difficulty(), S.Common.Col.DifficultyTitle()), numCol("Log-loss"),
	}
}

func tournamentRow(r tourLine) ui.Node {
	return ui.Trow(
		textCell(str(r.date), ui.NoWrap()),
		nameCell(tourHref(r.id), orUntitled(str(r.name))),
		textCell(tourType(str(r.typ))),
		numCell(fmt.Sprintf("%.*f", weightDigits, num(r.weight))),
		numCell(itoa(count(r.nTeams))),
		numCell(itoa(count(r.nQ))),
		numCell(blank(r.truedl, 1)),
		numCell(opt(r.llcf, llDigits)),
	)
}

func (s *site) releasesPage() (string, []ui.Item, error) {
	rows, err := s.q.releaseLines()
	if err != nil {
		return "", nil, err
	}
	trs := make([]ui.Item, len(rows))
	for i, r := range rows {
		trs[i] = releaseRow(r)
	}
	cols := []ui.Item{numCol("№"), nameCol(S.Common.Col.Date()), numCol(S.Releases.Col.Players()),
		numCol(S.Releases.Col.Teams()), numCol(S.Releases.Col.Shift()), numCol(S.Releases.Col.ShiftCum())}
	lede := S.Releases.Lede(itoa(int64(len(s.rels))))
	if s.model != "" {
		lede += " " + S.Common.ModelVersion(s.model) + "."
	}
	return S.Common.Nav.Releases(), []ui.Item{head(ui.Text(lede)), ui.Section(table(cols, trs...))}, nil
}

func releaseRow(r releaseLine) ui.Node {
	return ui.Trow(
		numCell(itoa(r.id), ui.Faint()),
		nameCell(releaseHref(r.id), r.date),
		numCell(itoa(r.nPlayers)),
		numCell(itoa(r.nTeams)),
		numCell(fmt.Sprintf("%+.4f", r.shift), ui.Faint()),
		numCell(fmt.Sprintf("%+.4f", r.cum), ui.Faint()),
	)
}

func (s *site) anomalyTable(page int) (string, []ui.Item, error) {
	p := paginate(s.nAnomaly, page, "/anomaly/", "/anomaly/page/")
	rows, err := s.q.anomalies(p.lo, p.hi())
	if err != nil {
		return "", nil, err
	}
	trs := make([]ui.Item, len(rows))
	for i, r := range rows {
		trs[i] = anomalyRow(r)
	}
	sec := append([]ui.Item{s.anomalyExplainer()}, p.framed(table(anomalyCols(), trs...))...)
	return p.title(S.Anomaly.Title()), []ui.Item{head(), ui.Section(sec...)}, nil
}

func (s *site) anomalyExplainer() ui.Node {
	named := func(name, text string) ui.Node {
		return ui.Paragraph(ui.Inline(ui.Strong(ui.Text(name)), ui.Text(" "+text)))
	}
	return ui.Explainer(ui.Label(S.Anomaly.Explain.Summary()),
		ui.Paragraph(ui.Text(S.Anomaly.Explain.Gap())),
		named(S.Anomaly.Explain.ForecastName(), S.Anomaly.Explain.Forecast()),
		named(S.Anomaly.Explain.HardName(), S.Anomaly.Explain.Hard()),
		ui.Paragraph(ui.Text(S.Anomaly.Explain.Shrink())),
		ui.Paragraph(ui.Text(S.Anomaly.Explain.Count(spaced(s.nAnomaly), s.nAnomaly))),
	)
}

func anomalyCols() []ui.Item {
	return []ui.Item{numCol("№"), nameCol(S.Common.Col.Player()), numCol(S.Common.Col.Rating()),
		numCol(S.Common.Col.Place()), numCol(S.Anomaly.Col.Sync()), numCol(S.Anomaly.Col.InPerson()),
		numCol(S.Anomaly.Col.Forecast()), numCol(S.Anomaly.Col.Hard()), numCol(S.Anomaly.Col.Score())}
}

func anomalyRow(r anomalyLine) ui.Node {
	return ui.Trow(
		numCell(itoa(r.rank), ui.Faint()),
		nameCell(playerHref(r.id), playerName(r.name, r.id)),
		numCell(dispOpt(r.display)),
		numCell(intOpt(r.prank), ui.Faint()),
		numCell(fmt.Sprintf("%.0f", num(r.fOn))),
		numCell(fmt.Sprintf("%.0f", num(r.fIP))),
		numCell(fmt.Sprintf("×%.2f", math.Exp(num(r.fGap))), toneOf(num(r.fGap))),
		numCell(signed1(percent*num(r.mGap)), toneOf(num(r.mGap))),
		numCell(fmt.Sprintf("%.2f", num(r.k)), ui.Bold()),
	)
}

// provToggle is the provisional switch. It is off by default, so a reader with
// no script sees only settled ratings; site.js remembers the choice.
func provToggle(hidden int, firstPage int, page int) ui.Node {
	var note string
	switch {
	case hidden > 0:
		note = S.Common.Prov.Hidden(itoa(int64(hidden)))
	case firstPage > page:
		note = S.Common.Prov.FromPage(itoa(int64(firstPage)))
	}
	items := []ui.Item{ui.Text(S.Common.Prov.Toggle())}
	if note != "" {
		items = append(items, ui.Text(" "), ui.Count(ui.Text(note)))
	}
	return ui.Toggle(ui.ToggleProv, ui.Inline(items...))
}
