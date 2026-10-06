package main

import (
	"fmt"
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
	for i, w := range words {
		words[i] = `"` + w + `"*`
	}
	return strings.Join(words, " ")
}

func (s *site) searchPage(q string) (string, []ui.Item, error) {
	out := []ui.Item{head(S.Search.Title(), ui.Text(S.Search.Query(q)))}
	query := ftsQuery(q)
	if query == "" {
		return S.Search.PageTitle(q), append(out, ui.Empty(ui.Text(S.Search.Prompt()))), nil
	}

	var players []ui.Item
	if rows, err := s.db.Query(`select f.rowid, p.name, k.rank, k.display, p.games, p.last_date
		from x.player_fts f join x.player p on p.id = f.rowid
		left join x.player_rank k on k.player_id = f.rowid
		where player_fts match ? order by k.rank is null, k.rank, p.games desc limit 40`, query); err == nil {
		for rows.Next() {
			var id int64
			var name, last *string
			var rank, games *int64
			var display *float64
			if rows.Scan(&id, &name, &rank, &display, &games, &last) != nil {
				continue
			}
			players = append(players, ui.Trow(
				numCell(rankText(rank), ui.Faint()),
				nameCell(fmt.Sprintf("/players/%d/", id), str(name)),
				numCell(dispOpt(display), ui.Bold()),
				numCell(itoa(count(games))),
				textCell(str(last), ui.NoWrap(), ui.Faint()),
			))
		}
		rows.Close()
	}

	var teams []ui.Item
	if rows, err := s.db.Query(`select f.rowid, t.name, t.town, k.rank, k.display, t.games, t.last_date
		from x.team_fts f join x.team t on t.id = f.rowid
		left join x.team_rank k on k.team_id = f.rowid
		where team_fts match ? order by k.rank is null, k.rank, t.games desc limit 40`, query); err == nil {
		for rows.Next() {
			var id int64
			var name, tw, last *string
			var rank, games *int64
			var display *float64
			if rows.Scan(&id, &name, &tw, &rank, &display, &games, &last) != nil {
				continue
			}
			teams = append(teams, ui.Trow(
				numCell(rankText(rank), ui.Faint()),
				nameCell(fmt.Sprintf("/teams/%d/", id), str(name), town(str(tw))),
				numCell(dispOpt(display), ui.Bold()),
				numCell(itoa(count(games))),
				textCell(str(last), ui.NoWrap(), ui.Faint()),
			))
		}
		rows.Close()
	}

	if len(players) == 0 && len(teams) == 0 {
		return S.Search.PageTitle(q), append(out, ui.Empty(ui.Text(S.Search.Nothing()))), nil
	}
	if len(players) > 0 {
		table := append([]ui.Item{ui.Scroll(), ui.Trow(
			hcell(S.Common.Col.Place(), ui.Num()),
			hcell(S.Common.Col.Player(), ui.PinCol()),
			hcell(S.Common.Col.Rating(), ui.Num()),
			hcell(S.Search.Col.Games(), ui.Num()),
			hcell(S.Search.Col.Last()),
		)}, players...)
		out = append(out, ui.Section(ui.Subhead(ui.Text(S.Search.Players(itoa(int64(len(players)))))), ui.Table(table...)))
	}
	if len(teams) > 0 {
		table := append([]ui.Item{ui.Scroll(), ui.Trow(
			hcell(S.Common.Col.Place(), ui.Num()),
			hcell(S.Common.Col.Team(), ui.PinCol()),
			hcell(S.Common.Col.Rating(), ui.Num()),
			hcell(S.Search.Col.Tournaments(), ui.Num()),
			hcell(S.Search.Col.Last()),
		)}, teams...)
		out = append(out, ui.Section(ui.Subhead(ui.Text(S.Search.Teams(itoa(int64(len(teams)))))), ui.Table(table...)))
	}
	return S.Search.PageTitle(q), out, nil
}

func rankText(r *int64) string {
	if r == nil {
		return "—"
	}
	return itoa(*r)
}
