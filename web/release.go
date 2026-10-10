package main

import (
	"net/http"
	"strconv"

	"ratingweb/ui"
)

// A past release: its team and player tables as they were published, read
// from the fit's own rows for that release and ranked the way webindex ranks
// the last one.

func (st *store) teamRanksAt(rel int64, lo, hi int) ([]teamRank, error) {
	return all[teamRank](st.db, `select row_number() over (order by r.display desc, r.team_id) + ?, r.team_id, t.name, t.town,
		r.display, null, r.evidence, r.change, 0 from team_ratings r left join x.team t on t.id = r.team_id
		where r.release_id = ? order by r.display desc, r.team_id limit ? offset ?`, lo, rel, hi-lo, lo)
}

func (st *store) playerRanksAt(rel int64, lo, hi int) ([]playerRank, error) {
	c := st.sc
	return all[playerRank](st.db, `with r as (select player_id, coalesce(display, ? + ? * theta) as display, ? * sigma as sd,
		games_365, delta, sigma > ? as prov from player_ratings where release_id = ?)
		select row_number() over (order by r.display - ? * r.sd desc, r.player_id) + ?, r.player_id, p.name, r.display, r.sd,
		r.games_365, r.delta, r.prov from r left join x.player p on p.id = r.player_id
		order by r.display - ? * r.sd desc, r.player_id limit ? offset ?`,
		c.aPlayer, c.bPlayer, abs(c.bPlayer), c.provSigma, rel, c.rankSigmas, lo, c.rankSigmas, hi-lo, lo)
}

func (st *store) countAt(table string, rel int64) (int, error) {
	return st.number("select count(*) from "+table+" where release_id = ?", rel)
}

// releaseRoute is a page under /releases/id/: the team table, or with
// players/ the player table, each with its pages.
func releaseRoute(segs []string) page {
	rest := segs[2:]
	players := len(rest) > 0 && rest[0] == "players"
	if players {
		rest = rest[1:]
	}
	n := 1
	switch {
	case len(rest) == 2 && rest[0] == "page":
		n = pageNo(rest[1])
	case len(rest) != 0:
		return nil
	}
	return func(s *site, _ *http.Request, segs []string) (string, []ui.Item, error) {
		id, err := strconv.ParseInt(segs[1], decimal, idBits)
		i, ok := s.relIdx[id]
		if err != nil || !ok {
			return "", nil, errNotFound
		}
		if players {
			return s.pastPlayers(s.rels[i], n)
		}
		return s.pastTeams(s.rels[i], n)
	}
}

func releaseHref(id int64) string { return "/releases/" + itoa(id) + "/" }

func (s *site) pastTeams(r release, page int) (string, []ui.Item, error) {
	n, err := s.q.countAt("team_ratings", r.id)
	if err != nil {
		return "", nil, err
	}
	base := releaseHref(r.id)
	p := paginate(n, page, base, base+"page/")
	rows, err := s.q.teamRanksAt(r.id, p.lo, p.hi())
	lines := make([]rankLine, len(rows))
	for i, x := range rows {
		lines[i] = s.teamLine(x)
	}
	head := pastHead(S.Teams.Lede(r.date, n), base+"players/", S.Releases.Players())
	t := rankTable(s.teamCols(), lines, true, "")
	return p.title(S.Releases.Title(r.date)), []ui.Item{head, ui.Section(p.framed(t)...)}, err
}

func (s *site) pastPlayers(r release, page int) (string, []ui.Item, error) {
	n, err := s.q.countAt("player_ratings", r.id)
	if err != nil {
		return "", nil, err
	}
	base := releaseHref(r.id)
	p := paginate(n, page, base+"players/", base+"players/page/")
	rows, err := s.q.playerRanksAt(r.id, p.lo, p.hi())
	lines := make([]rankLine, len(rows))
	for i, x := range rows {
		lines[i] = playerLine(x)
		lines[i].ranked = true
	}
	head := pastHead(S.Players.Lede(r.date, n), base, S.Releases.Teams())
	t := rankTable(playerCols(s.q.sc.rankSigmas), lines, true, "")
	return p.title(S.Releases.Title(r.date)), []ui.Item{head, ui.Section(p.framed(t)...)}, err
}

// pastHead is a past table's line: its release and size, and the way to the
// release's other table.
func pastHead(lede, other, otherName string) ui.Node {
	return head(ui.Text(lede+" "), link(other, otherName), ui.Text("."))
}
