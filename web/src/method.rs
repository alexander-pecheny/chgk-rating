// The methodology page: DESIGN.md sections 1–6 in the plain register of the
// rating site, with the PDF report as the formal source. Written in English;
// the owner translates by hand.
pub const BODY: &str = r#"<header class="page-head"><h1>Method</h1><p class="muted">How the rating is computed, in short. The full rules with formulas are in the <a href="/report.pdf">report (PDF)</a>.</p></header>
<section class="section"><h2 class="subhead">What a team rating is</h2>
<p>A team rating is its demonstrated strength on a stable scale of points. It changes only when the team plays. Time off neither adds nor removes anything.</p>
<p>Each update is driven by places in the tournament, not by the number of questions answered. Against every other team in the field we compute the expected chance of finishing above it, 1 / (1 + 10<sup>(their strength − yours) / 1200</sup>). The actual outcome is 1 if you finished above that team and 0 if below. Pairs that shared a place are skipped in both sums. The average over the field of actual minus expected is the surprise of the result, and the rating moves by 0.4 × tournament weight × 1200 × surprise.</p>
<p>A team that finishes first never loses points. Every pair it is above contributes a non-negative amount, and a shared first place contributes nothing negative. This follows from the update itself, not from an extra rule. The score margin plays no part.</p>
</section>
<section class="section"><h2 class="subhead">Lineup</h2>
<p>A change of lineup does not move the rating by itself. Besides the rating, each team carries a <em>usual lineup</em>: the strength of the lineups that earned that rating. In a tournament field the team stands with strength «rating + (today's lineup strength − usual lineup)». Guests raise the expectation, absentees lower it, and neither touches the rating number. After the tournament the usual lineup moves one percent towards today's lineup, so it follows real changes and ignores one-off ones.</p>
<p>Lineup strength is estimated from the individual ratings of the players at the table. It is the strength of a single player who would answer as many questions as the whole table together. Six equal players of strength R form a lineup of strength R. A short table is weaker. A strong player in a weak lineup lifts the estimate by more than they would lift the average.</p>
<p>A lineup that plays under another team's number (the database marks such a result as unrated) stands in the field as an opponent for everyone else but earns and loses nothing for its own team. Its players are updated in the usual way.</p>
</section>
<section class="section"><h2 class="subhead">Individual rating</h2>
<p>Player strength is estimated from a question model. Each question has a difficulty, each player has a strength and an uncertainty, and the chance that a team answers a question is built from the strengths of those at the table. Question difficulty is read from the share of lineups that answered it: it is set so that the model predicts exactly the observed share. This is what makes packages from different years and levels comparable.</p>
<p>Individual numbers are published as they are, with their uncertainty, with no ladder rules and no rounding to grades. A player with no history enters at the average strength of a newcomer with a wide uncertainty.</p>
</section>
<section class="section"><h2 class="subhead">Scale</h2>
<p>At every release the scale is anchored to the population: the mean strength of players with at least eight rated tournaments in the past year is set to zero. This shift is subtracted from every player. The team scale rests on the same anchor, because both lineup strength and a new team's first rating are computed from players. The accumulated shift is shown on the <a href="/releases/">releases</a> page.</p>
</section>
<section class="section"><h2 class="subhead">Provisional ratings</h2>
<p>A team rating counts as established after five rated tournaments. Before that it is marked provisional and left out of the ranked tables. This is not a penalty or a separate league. It is an honest «we do not know yet».</p>
<p>Players carry a ± next to the rating: the root-mean-square uncertainty of the estimate. A rating with a large one is provisional. The width is driven by the player's own volatility. A steady, predictable history squeezes it to the floor no matter how much time passes, and a run of results unlike the past opens it again. The calendar alone does not blur a rating.</p>
</section>
<section class="section"><h2 class="subhead">Forecast</h2>
<p>The forecast for a tournament is separate from the rating and takes no part in the update. A team's forecast strength is the strength of the lineup it fielded, computed from individual ratings, and the forecast places are the order of the field by that strength. About tomorrow, the lineup knows more than the accumulated team rating: on in-person tournaments of the last two seasons this forecast beats both the team rating and the forecast of the old system.</p>
<p>The tournament page shows the forecast next to the result. Forecast quality is measured by Spearman rank correlation and compared with the old system.</p>
</section>
<section class="section"><h2 class="subhead">Package difficulty</h2>
<p>Package difficulty is a single estimate, shared by all tournaments, of how hard the questions were: the higher the number, the fewer questions teams of one and the same strength would answer. It comes from the same question model as the player ratings, so packages of different years, formats and fields compare directly.</p>
</section>
<section class="section"><h2 class="subhead">Which tournaments count</h2>
<p>Rated tournaments from 1 January 2013 count. The format enters as a weight: in-person tournaments 1, strict synchronous 2/3, synchronous 1/2. Asynchronous tournaments are never rated. Marathons are excluded.</p>
<p>The tables include teams and players that played at least one rated tournament in the last 365 days.</p>
</section>
<section class="section"><h2 class="subhead">Publication</h2>
<p>Processing is online and chronological: tournaments in date order, releases on Thursdays. A published number is never rewritten later. Full recomputations happen offline, to tune constants and to run history for the first time.</p>
</section>
<section class="section"><h2 class="subhead">Further reading</h2>
<p>Formulas, constant calibration and acceptance tests are in the <a href="/report.pdf">report (PDF)</a>.</p></section>"#;
