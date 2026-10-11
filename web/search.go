package main

import (
	"errors"
	"strings"
	"unicode"

	"ratingweb/ui"
)

// ftsQuery turns what a reader typed into an FTS5 query. FTS5 reads its query
// as a small language of its own, so a name typed with a quote or a hyphen
// would be a syntax error. Each word is quoted and given a prefix star, which
// is what a reader typing half a surname expects.
func ftsQuery(q string) string {
	words := strings.FieldsFunc(q, func(r rune) bool { return !unicode.IsLetter(r) && !unicode.IsNumber(r) })
	// The index holds ё as е, as most people type it.
	fold := strings.NewReplacer("ё", "е", "Ё", "Е")
	for i, w := range words {
		words[i] = `"` + fold.Replace(w) + `"*`
	}
	return strings.Join(words, " ")
}

func (s *site) searchPage(q string) (string, []ui.Item, error) {
	out := []ui.Item{head(ui.Text(S.Search.Query(q)))}
	query := ftsQuery(q)
	if query == "" {
		return S.Search.PageTitle(q), append(out, ui.Empty(ui.Text(S.Search.Prompt()))), nil
	}
	players, err := s.q.findPlayers(query)
	teams, err2 := s.q.findTeams(query)
	tours, err3 := s.q.findTournaments(query)
	if err = errors.Join(err, err2, err3); err != nil {
		return "", nil, err
	}
	if len(players) == 0 && len(teams) == 0 && len(tours) == 0 {
		return S.Search.PageTitle(q), append(out, ui.Empty(ui.Text(S.Search.Nothing()))), nil
	}
	if len(players) > 0 {
		out = append(out, hits(S.Search.Players, S.Common.Col.Player(), S.Search.Col.Games(), players, playerHit))
	}
	if len(teams) > 0 {
		out = append(out, hits(S.Search.Teams, S.Common.Col.Team(), S.Search.Col.Tournaments(), teams, teamHit))
	}
	if len(tours) > 0 {
		out = append(out, tourHits(tours))
	}
	return S.Search.PageTitle(q), out, nil
}

// tourHits are the tournaments found, as the tournaments page lists them.
func tourHits(found []tourFound) ui.Node {
	rows := make([]ui.Item, len(found))
	for i, t := range found {
		rows[i] = ui.Trow(append(tourCells(str(t.date), t.id, str(t.name), str(t.typ), true),
			numCell(intOpt(t.teams)))...)
	}
	cols := append(tourCols(), numCol(S.Tournaments.Col.Teams()))
	return section(S.Search.Tournaments(itoa(int64(len(found)))), table(cols, rows...))
}

// hits are the search results of one kind, as a titled table.
func hits(title func(string) string, who, games string, found []found, cell func(found) ui.Node) ui.Node {
	rows := make([]ui.Item, len(found))
	for i, f := range found {
		rows[i] = ui.Trow(
			numCell(intOpt(f.rank), ui.Faint()),
			cell(f),
			numCell(dispOpt(f.display), ui.Bold()),
			numCell(itoa(count(f.games))),
			textCell(str(f.last), ui.NoWrap(), ui.Faint()),
		)
	}
	cols := []ui.Item{numCol(S.Common.Col.Place()), nameCol(who), numCol(S.Common.Col.Rating()), numCol(games), hcell(S.Search.Col.Last())}
	return section(title(itoa(int64(len(found)))), table(cols, rows...))
}

func playerHit(f found) ui.Node { return nameCell(playerHref(f.id), playerName(f.name, f.id)) }

func teamHit(f found) ui.Node {
	return nameCell(teamHref(f.id), teamName(f.name, f.id), town(str(f.town)))
}
