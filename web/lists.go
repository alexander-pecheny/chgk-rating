package main

import (
	"fmt"
	"math"

	"ratingweb/ui"
)

// The listings: the two ranking tables, the tournaments, the releases and the
// anomaly scores. Each is a page of pageSize rows with a pager over and under.

func (s *site) teamTable(page int) (string, []ui.Item, error) {
	pages := pagesOf(s.nTeams)
	page = min(max(page, 1), pages)
	var firm int
	_ = s.db.QueryRow("select count(*) from x.team_rank where provisional = 0").Scan(&firm)
	lo := (page - 1) * pageSize
	rows, err := s.db.Query(`select k.rank, k.team_id, t.name, t.town, k.display, k.sigma, k.evidence, k.change,
		k.provisional from x.team_rank k left join x.team t on t.id = k.team_id
		where k.rank > ? and k.rank <= ? order by k.rank`, lo, lo+pageSize)
	if err != nil {
		return "", nil, err
	}
	defer rows.Close()
	type row struct {
		rank, id            int64
		name, town          *string
		display, sigma, evi *float64
		change              *float64
		prov                bool
	}
	var rs []row
	for rows.Next() {
		var r row
		if err := rows.Scan(&r.rank, &r.id, &r.name, &r.town, &r.display, &r.sigma, &r.evi, &r.change, &r.prov); err != nil {
			return "", nil, err
		}
		rs = append(rs, r)
	}
	if err := rows.Err(); err != nil {
		return "", nil, err
	}

	section := []ui.Item{}
	// The switch only matters when some team is provisional.
	if firm < s.nTeams {
		hidden := 0
		for _, r := range rs {
			if r.prov {
				hidden++
			}
		}
		section = append(section, provToggle(hidden, firm/pageSize+1, page))
	}
	section = append(section, pager(page, pages, "/", "/page/"))
	spread := hcell("±", ui.Num(), ui.Title(S.Common.Col.SigmaTitle()))
	if s.d.s2 {
		spread = hcell(S.Teams.Col.Results(), ui.Num(), ui.Title(S.Teams.Col.ResultsTitle()))
	}
	table := []ui.Item{ui.Scroll(), ui.Trow(
		hcell(S.Common.Col.Place(), ui.Num()),
		hcell(S.Common.Col.Team(), ui.PinCol()),
		hcell(S.Common.Col.Rating(), ui.Num()),
		spread,
		hcell(S.Common.Col.Change(), ui.Num()),
	)}
	split := lo >= firm
	for _, r := range rs {
		if r.prov && !split {
			split = true
			table = append(table, ui.Trow(ui.SplitRow(), ui.ProvRow(), textCell(S.Teams.ProvSplit(), ui.Colspan("5"))))
		}
		name := str(r.name)
		if name == "" {
			name = S.Common.TeamFallback(itoa(r.id))
		}
		rank := itoa(r.rank)
		var badge ui.Item
		if r.prov {
			rank, badge = "—", provBadge()
		}
		spreadCell := numCell(disp(num(r.sigma)), ui.Faint())
		if s.d.s2 {
			spreadCell = numCell(fmt.Sprintf("%.0f", num(r.evi)), ui.Faint())
		}
		tr := []ui.Item{
			numCell(rank),
			nameCell(fmt.Sprintf("/teams/%d/", r.id), name, badge, town(str(r.town))),
			numCell(disp(num(r.display)), ui.Bold()),
			spreadCell,
			changeCell(r.change),
		}
		if r.prov {
			tr = append(tr, ui.ProvRow())
		}
		table = append(table, ui.Trow(tr...))
	}
	section = append(section, ui.Table(table...), pager(page, pages, "/", "/page/"))

	title := S.Common.Nav.Teams()
	if page > 1 {
		title = S.Common.PageTitle(title, itoa(int64(page)))
	}
	return title, []ui.Item{
		head(S.Common.Nav.Teams(),
			ui.Text(S.Teams.Lede(s.releaseDate, s.nTeams)+" "),
			link("/method/", S.Teams.How()),
			ui.Text("."),
		),
		ui.Section(section...),
	}, nil
}

func (s *site) playerTable(page int) (string, []ui.Item, error) {
	pages := pagesOf(s.nPlayers)
	page = min(max(page, 1), pages)
	var firm int
	_ = s.db.QueryRow("select count(*) from x.player_rank where provisional = 0").Scan(&firm)
	lo := (page - 1) * pageSize
	rows, err := s.db.Query(`select k.rank, k.player_id, p.name, k.display, k.sigma, k.games, k.change,
		k.provisional from x.player_rank k left join x.player p on p.id = k.player_id
		where k.rank > ? and k.rank <= ? order by k.rank`, lo, lo+pageSize)
	if err != nil {
		return "", nil, err
	}
	defer rows.Close()
	type row struct {
		rank, id       int64
		name           *string
		display, sigma *float64
		games          *int64
		change         *float64
		prov           bool
	}
	var rs []row
	for rows.Next() {
		var r row
		if err := rows.Scan(&r.rank, &r.id, &r.name, &r.display, &r.sigma, &r.games, &r.change, &r.prov); err != nil {
			return "", nil, err
		}
		rs = append(rs, r)
	}
	if err := rows.Err(); err != nil {
		return "", nil, err
	}

	hidden := 0
	for _, r := range rs {
		if r.prov {
			hidden++
		}
	}
	firstProv := 0
	if firm < s.nPlayers {
		firstProv = firm/pageSize + 1
	}
	section := []ui.Item{provToggle(hidden, firstProv, page), pager(page, pages, "/players/", "/players/page/")}
	table := []ui.Item{ui.Scroll(), ui.Trow(
		hcell(S.Common.Col.Place(), ui.Num()),
		hcell(S.Common.Col.Player(), ui.PinCol()),
		hcell(S.Common.Col.Rating(), ui.Num()),
		hcell("±", ui.Num(), ui.Title(S.Common.Col.SigmaTitle())),
		hcell(S.Players.Col.GamesYear(), ui.Num()),
		hcell(S.Common.Col.Change(), ui.Num()),
	)}
	split := lo >= firm
	for _, r := range rs {
		if r.prov && !split {
			split = true
			table = append(table, ui.Trow(ui.SplitRow(), ui.ProvRow(), textCell(S.Players.ProvSplit(), ui.Colspan("6"))))
		}
		name := str(r.name)
		if name == "" {
			name = S.Common.PlayerFallback(itoa(r.id))
		}
		rank := itoa(r.rank)
		var badge ui.Item
		if r.prov {
			rank, badge = "—", provBadge()
		}
		tr := []ui.Item{
			numCell(rank),
			nameCell(fmt.Sprintf("/players/%d/", r.id), name, badge),
			numCell(disp(num(r.display)), ui.Bold()),
			numCell(disp(num(r.sigma)), ui.Faint()),
			numCell(itoa(count(r.games))),
			changeCell(r.change),
		}
		if r.prov {
			tr = append(tr, ui.ProvRow())
		}
		table = append(table, ui.Trow(tr...))
	}
	section = append(section, ui.Table(table...), pager(page, pages, "/players/", "/players/page/"))

	title := S.Common.Nav.Players()
	if page > 1 {
		title = S.Common.PageTitle(title, itoa(int64(page)))
	}
	return title, []ui.Item{
		head(S.Common.Nav.Players(), ui.Text(S.Players.Lede(s.releaseDate, s.nPlayers, itoa(int64(firm))))),
		ui.Section(section...),
	}, nil
}

func (s *site) tournamentTable(page int) (string, []ui.Item, error) {
	var n int
	_ = s.db.QueryRow("select count(*) from x.tournament").Scan(&n)
	pages := pagesOf(n)
	page = min(max(page, 1), pages)
	rows, err := s.db.Query(`select id, name, date, type, weight, n_teams, n_q, truedl, ll_cf from x.tournament
		order by date desc, id desc limit ? offset ?`, pageSize, (page-1)*pageSize)
	if err != nil {
		return "", nil, err
	}
	defer rows.Close()
	table := []ui.Item{ui.Scroll(), ui.Trow(
		hcell(S.Common.Col.Date()),
		hcell(S.Common.Col.Tournament(), ui.PinCol()),
		hcell(S.Common.Col.Type()),
		hcell(S.Tournaments.Col.Weight(), ui.Num()),
		hcell(S.Tournaments.Col.Teams(), ui.Num()),
		hcell(S.Tournaments.Col.Questions(), ui.Num()),
		hcell(S.Common.Col.Difficulty(), ui.Num(), ui.Title(S.Common.Col.DifficultyTitle())),
		hcell("Log-loss", ui.Num()),
	)}
	for rows.Next() {
		var id int64
		var name, date, typ *string
		var weight, truedl, llcf *float64
		var nTeams, nQ *int64
		if err := rows.Scan(&id, &name, &date, &typ, &weight, &nTeams, &nQ, &truedl, &llcf); err != nil {
			return "", nil, err
		}
		table = append(table, ui.Trow(
			textCell(str(date), ui.NoWrap()),
			nameCell(fmt.Sprintf("/tournaments/%d/", id), orUntitled(str(name))),
			textCell(tourType(str(typ))),
			numCell(fmt.Sprintf("%.2f", num(weight))),
			numCell(itoa(count(nTeams))),
			numCell(itoa(count(nQ))),
			numCell(blank(truedl, 1)),
			numCell(opt(llcf, 3)),
		))
	}
	if err := rows.Err(); err != nil {
		return "", nil, err
	}
	title := S.Common.Nav.Tournaments()
	if page > 1 {
		title = S.Common.PageTitle(title, itoa(int64(page)))
	}
	return title, []ui.Item{
		head(S.Common.Nav.Tournaments(), ui.Text(S.Tournaments.Lede(n))),
		ui.Section(
			pager(page, pages, "/tournaments/", "/tournaments/page/"),
			ui.Table(table...),
			pager(page, pages, "/tournaments/", "/tournaments/page/"),
		),
	}, nil
}

func (s *site) releasesPage() (string, []ui.Item, error) {
	rows, err := s.db.Query("select id, date, n_players, n_teams, shift, shift_cum from x.release_ order by date desc, id desc")
	if err != nil {
		return "", nil, err
	}
	defer rows.Close()
	table := []ui.Item{ui.Scroll(), ui.Trow(
		hcell("№", ui.Num()),
		hcell(S.Common.Col.Date()),
		hcell(S.Releases.Col.Players(), ui.Num()),
		hcell(S.Releases.Col.Teams(), ui.Num()),
		hcell(S.Releases.Col.Shift(), ui.Num()),
		hcell(S.Releases.Col.ShiftCum(), ui.Num()),
	)}
	for rows.Next() {
		var id, nPlayers, nTeams int64
		var date string
		var shift, cum float64
		if err := rows.Scan(&id, &date, &nPlayers, &nTeams, &shift, &cum); err != nil {
			return "", nil, err
		}
		table = append(table, ui.Trow(
			numCell(itoa(id), ui.Faint()),
			textCell(date, ui.NoWrap()),
			numCell(itoa(nPlayers)),
			numCell(itoa(nTeams)),
			numCell(fmt.Sprintf("%+.4f", shift), ui.Faint()),
			numCell(fmt.Sprintf("%+.4f", cum), ui.Faint()),
		))
	}
	if err := rows.Err(); err != nil {
		return "", nil, err
	}
	return S.Common.Nav.Releases(), []ui.Item{
		head(S.Common.Nav.Releases(), ui.Text(S.Releases.Lede(itoa(int64(len(s.rels)))))),
		ui.Section(ui.Table(table...)),
	}, nil
}

func (s *site) anomalyTable(page int) (string, []ui.Item, error) {
	pages := pagesOf(s.nAnomaly)
	page = min(max(page, 1), pages)
	lo := (page - 1) * pageSize
	rows, err := s.db.Query(`select a.rank, a.player_id, p.name, k.rank, k.display, a.k, a.m_gap, a.m_on,
		a.m_ip, a.f_gap, a.f_on, a.f_ip from x.anomaly a join x.player p on p.id = a.player_id
		left join x.player_rank k on k.player_id = a.player_id
		where a.rank > ? and a.rank <= ? order by a.rank`, lo, lo+pageSize)
	if err != nil {
		return "", nil, err
	}
	defer rows.Close()
	table := []ui.Item{ui.Scroll(), ui.Trow(
		hcell("№", ui.Num()),
		hcell(S.Common.Col.Player(), ui.PinCol()),
		hcell(S.Common.Col.Rating(), ui.Num()),
		hcell(S.Common.Col.Place(), ui.Num()),
		hcell(S.Anomaly.Col.Sync(), ui.Num()),
		hcell(S.Anomaly.Col.InPerson(), ui.Num()),
		hcell(S.Anomaly.Col.Forecast(), ui.Num()),
		hcell(S.Anomaly.Col.Hard(), ui.Num()),
		hcell(S.Anomaly.Col.Score(), ui.Num()),
	)}
	for rows.Next() {
		var rank, id int64
		var name *string
		var prank *int64
		var display, k, mGap, mOn, mIP, fGap, fOn, fIP *float64
		if err := rows.Scan(&rank, &id, &name, &prank, &display, &k, &mGap, &mOn, &mIP, &fGap, &fOn, &fIP); err != nil {
			return "", nil, err
		}
		nm := str(name)
		if nm == "" {
			nm = S.Common.PlayerFallback(itoa(id))
		}
		placeText := "—"
		if prank != nil {
			placeText = itoa(*prank)
		}
		table = append(table, ui.Trow(
			numCell(itoa(rank), ui.Faint()),
			nameCell(fmt.Sprintf("/players/%d/", id), nm),
			numCell(dispOpt(display)),
			numCell(placeText, ui.Faint()),
			numCell(fmt.Sprintf("%.0f", num(fOn))),
			numCell(fmt.Sprintf("%.0f", num(fIP))),
			numCell(fmt.Sprintf("×%.2f", math.Exp(num(fGap))), toneOf(num(fGap))),
			numCell(signed1(100*num(mGap)), toneOf(num(mGap))),
			numCell(fmt.Sprintf("%.2f", num(k)), ui.Bold()),
		))
	}
	if err := rows.Err(); err != nil {
		return "", nil, err
	}
	title := S.Anomaly.Title()
	if page > 1 {
		title = S.Common.PageTitle(title, itoa(int64(page)))
	}
	return title, []ui.Item{
		head(S.Anomaly.Title()),
		ui.Section(
			ui.Explainer(ui.Label(S.Anomaly.Explain.Summary()),
				ui.Paragraph(ui.Text(S.Anomaly.Explain.Gap())),
				ui.Paragraph(ui.Inline(ui.Strong(ui.Text(S.Anomaly.Explain.ForecastName())), ui.Text(" "+S.Anomaly.Explain.Forecast()))),
				ui.Paragraph(ui.Inline(ui.Strong(ui.Text(S.Anomaly.Explain.HardName())), ui.Text(" "+S.Anomaly.Explain.Hard()))),
				ui.Paragraph(ui.Text(S.Anomaly.Explain.Shrink())),
				ui.Paragraph(ui.Text(S.Anomaly.Explain.Count(spaced(s.nAnomaly), s.nAnomaly))),
			),
			pager(page, pages, "/anomaly/", "/anomaly/page/"),
			ui.Table(table...),
			pager(page, pages, "/anomaly/", "/anomaly/page/"),
		),
	}, nil
}

// space is the gap before an inline mark, or nothing when there is no mark.
func space(mark ui.Item) ui.Item {
	if mark == nil {
		return nil
	}
	return ui.Text(" ")
}
