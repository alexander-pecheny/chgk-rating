package main

import (
	"fmt"
	"sort"

	"ratingweb/ui"
)

// A tournament not yet played whose lineups are already listed: the fit
// forecast it from the players as they stood at the last release, and read
// its expected scores on the questions of a pack already played.

// upcomingHead is the last day of results the forecast saw and which pack
// lent it questions.
type upcomingHead struct {
	asOf            string
	refID           *int64
	refName         *string
	refDate, rule   *string
	refDL, announce *float64
}

func (r *upcomingHead) fields() []any {
	return []any{&r.asOf, &r.refID, &r.refName, &r.refDate, &r.rule, &r.refDL, &r.announce}
}

// upcoming is the tournament's forecast head; not found for one already played.
func (st *store) upcoming(id int64) (upcomingHead, bool, error) {
	return one[upcomingHead](st.db, `select u.as_of, u.ref_id, t.name, t.date, u.ref_rule, u.ref_truedl,
		u.announced from x.upcoming u left join x.tournament t on t.id = u.ref_id where u.tournament_id = ?`, id)
}

// upcomingRow is one listed team: where the forecast puts it and what it is
// expected to take.
type upcomingRow struct {
	team                   int64
	name, entered          *string
	place, expected, power *float64
}

func (r *upcomingRow) fields() []any {
	return []any{&r.team, &r.name, &r.entered, &r.place, &r.expected, &r.power}
}

func (st *store) upcomingField(id int64) ([]upcomingRow, error) {
	rows, err := all[upcomingRow](st.db, `select u.team_id, m.name, u.name, u.forecast_place, u.e_score, u.s_r
		from x.upcoming_team u left join x.team m on m.id = u.team_id
		where u.tournament_id = ? order by u.forecast_place, u.team_id`, id)
	for i := range rows {
		rows[i].power = mapped(rows[i].power, st.sc.team)
	}
	return rows, err
}

// upcomingLineups are the players each team listed, strongest first, at the
// numbers the forecast read.
func (st *store) upcomingLineups(id int64) (map[int64][]member, error) {
	rows, err := all[memberScan](st.db, `select team_id, player_id, name, theta, null, phi, sigma, tournament_id
		from x.upcoming_player where tournament_id = ?`, id)
	out := map[int64][]member{}
	for _, r := range rows {
		out[r.team] = append(out[r.team], st.member(r))
	}
	for _, v := range out {
		sort.SliceStable(v, func(i, j int) bool { return stronger(v[i], v[j]) })
	}
	return out, err
}

func (s *site) upcomingPage(id int64, t tourHead, u upcomingHead) ([]ui.Item, error) {
	rows, err := s.q.upcomingField(id)
	if err != nil {
		return nil, err
	}
	rosters, err := s.q.upcomingLineups(id)
	if err != nil {
		return nil, err
	}
	weight := fmt.Sprintf("%.2f", num(t.weight))
	head := ui.Pagehead(ui.Byline(ui.Text(S.Tournament.Byline(str(t.date), tourType(str(t.typ)), weight))))
	stats := []ui.Item{stat(S.Tournament.Stat.Teams(), itoa(count(t.nTeams)))}
	questions := dash
	if count(t.nQ) > 0 {
		questions = itoa(*t.nQ)
	}
	stats = append(stats, stat(S.Tournament.Stat.Questions(), questions))
	if u.announce != nil {
		stats = append(stats, stat(S.Tournament.Upcoming.Announced(), fmt.Sprintf("%.1f", *u.announce)))
	}
	about := []ui.Item{ui.Paragraph(ui.Text(S.Tournament.Upcoming.Lede(u.asOf))), referenceNote(u)}
	if u.refID != nil {
		about = append(about, ui.Paragraph(ui.Text(S.Tournament.Upcoming.Caveat())))
	}
	body := append(about, upcomingTable(rows, rosters)...)
	return []ui.Item{head, ui.Stats(stats...), section(S.Tournament.Upcoming.Title(), body...)}, nil
}

// referenceNote names the pack the expected scores were read on, or says
// there was none.
func referenceNote(u upcomingHead) ui.Node {
	if u.refID == nil {
		return ui.Paragraph(ui.Text(S.Tournament.Upcoming.NoRef()))
	}
	dl := blank(u.refDL, 1)
	pre, post := S.Tournament.Upcoming.Series(), S.Tournament.Upcoming.RefSeries(str(u.refDate), dl)
	if str(u.rule) == "difficulty" {
		pre, post = S.Tournament.Upcoming.Near(), S.Tournament.Upcoming.RefNear(str(u.refDate), dl, blank(u.announce, 1))
	}
	return ui.Paragraph(ui.Inline(ui.Text(pre), link(tourHref(*u.refID), orUntitled(str(u.refName))), ui.Text(post)))
}

func upcomingTable(rows []upcomingRow, rosters map[int64][]member) []ui.Item {
	var out []ui.Item
	if len(rosters) > 0 {
		out = append(out, ui.Toggle(ui.ToggleRoster, ui.Text(S.Tournament.Field.RosterToggle())))
	}
	header := []ui.Item{numCol(S.Tournament.Field.Forecast()), nameCol(S.Common.Col.Team()),
		numCol(S.Tournament.Upcoming.Expected(), S.Tournament.Upcoming.ExpectedTitle()),
		numCol(S.Tournament.Field.Lineup(), S.Tournament.Upcoming.LineupTitle())}
	trs := make([]ui.Item, len(rows))
	for i, r := range rows {
		trs[i] = ui.Trow(numCell(place(r.place)), ui.Cell(upcomingTeam(r, rosters[r.team])...),
			numCell(blank(r.expected, 0)), numCell(dispOpt(r.power), ui.Faint()))
	}
	return append(out, table(header, trs...))
}

// upcomingTeam is the team's own name, the name it entered under when that
// differs, and the lineup it listed.
func upcomingTeam(r upcomingRow, roster []member) []ui.Item {
	shown, entered := str(r.name), str(r.entered)
	if shown == "" {
		shown = entered
	}
	name := []ui.Item{link(teamHref(r.team), orDash(shown))}
	if entered != "" && fold(entered) != fold(shown) {
		name = append(name, ui.Text(" "), ui.Muted(ui.Text(S.Tournament.Field.PlayedAs(entered))))
	}
	cell := []ui.Item{ui.PinCol(), line(name...)}
	if len(roster) > 0 {
		cell = append(cell, ui.Rosteropen(), ui.Cellroster(lineup(roster)...))
	}
	return cell
}
