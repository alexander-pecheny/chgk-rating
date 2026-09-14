// =============================================================================
//  Appendix B. Rules for compiling the rating list (new system)
//  Build:  typst compile --font-path fonts report/report.typ report/report.pdf
// =============================================================================
//
//  TUNABLE QUANTITIES AND TEST FIGURES.
//
//  Every value below is filled in and matches config.toml and sections 7 and
//  8.1 of the specification on the release date. When a constant changes,
//  only this table is edited; the text pulls values by name.
//
//  Written in English; the owner translates into Russian by hand.
//
// -----------------------------------------------------------------------------

// -- Section B.1: coverage and weights ---------------------------------------
#let k_start      = [1 January 2013]
#let k_to         = [5 September 2026]
#let k_window     = [365]
#let k_w_in       = [1]
#let k_w_ssync    = [2/3]
#let k_w_sync     = [1/2]

// -- Section B.2: question model and individual rating -----------------------
#let k_b_bar      = [1.5]
#let k_s_b        = [2.0]
#let k_s_a        = [0.5]
#let k_a_min      = [0.3]
#let k_a_max      = [3.0]
#let k_b_min      = [−10]
#let k_b_max      = [12]
#let k_u_max      = [15]
#let k_newton     = [12]
#let k_info_scale = [0.46]
#let k_info_floor = [0.001]
#let k_theta_step = [1.5]
#let k_shrink     = [0.005]
#let k_mu_new     = [−1.0]
#let k_sigma_new  = [1.0]
#let k_tau_p      = [0.25]
#let k_v_min      = [0.05]
#let k_v_max      = [0.6]
#let k_v_eta      = [0.15]
#let k_n_est      = [8]
#let k_m_mean     = [1.056]
#let k_aff_mag    = [0.01]

// -- Section B.3: team model (Model S2) --------------------------------------
#let k_D           = [1200]
#let k_K           = [0.4]
#let k_Kq          = [0.1]
#let k_Kr          = [0.3]
#let k_scale       = [2454.8828]
#let k_zero        = [6000]
#let k_roster_bias = [−0.2427]
#let k_min_games   = [five]
#let k_tune_from   = [1 September 2018]
#let k_tune_to     = [29 August 2024]

// -- Section B.5: tournament difficulty --------------------------------------
#let k_theta_ref  = [0.2368]
#let k_ref_size   = [six]
#let k_band       = [251–500]

// -- Section B.6: published quantities ---------------------------------------
#let k_A_T        = [−7295.3562]
#let k_B_T        = [1.1904]
#let k_A_P        = [5543.6775]
#let k_B_P        = [4723.7898]
#let k_anchor_t   = [5 September 2019]
#let k_anchor_p   = [3 October 2019]
#let k_pos_hi     = [50]
#let k_pos_lo     = [1000]
#let k_ref_t_50   = [7028]
#let k_ref_t_1000 = [2992]
#let k_ref_p_50   = [12 865]
#let k_ref_p_1000 = [9778]
#let k_prov_sigma = [0.3]

// -- Section B.7: test results -----------------------------------------------
#let k_ll_cross   = [0.4900]
#let k_ll_2526    = [0.5161]
#let k_ll_quest   = [0.501]
#let k_sp_new     = [0.8363]
#let k_sp_old     = [0.8177]
#let k_sp_new_s   = [0.8194]
#let k_sp_old_s   = [0.7897]
#let k_ov_n       = [200]
#let k_ov_quest   = [117]
#let k_ov_fun     = [122]
#let k_ov_mutual  = [136]
#let k_ov_need    = [140]
#let k_corr_quest = [0.90]
#let k_corr_fun   = [0.79]
#let k_corr_games = [50]
#let k_infl       = [6.3%]
#let k_infl_band  = [±10%]
#let k_infl_from  = [2015]
#let k_infl_to    = [2026]
#let k_infl_n     = [11]
#let k_infl_skip  = [September 2021]
#let k_swap_n     = [286 139]

// -- Section B.8: transitional provisions ------------------------------------
#let k_passes     = [2]
#let k_warm_until = [1 January 2016]
#let k_warm_sigma = [0.5]

// =============================================================================
//  Layout
// =============================================================================

#set page(
  paper: "a4",
  margin: (top: 22mm, bottom: 22mm, left: 25mm, right: 20mm),
  numbering: "1",
  number-align: center,
)
#set text(font: "Literata Fix", lang: "en", size: 10.5pt, hyphenate: true)
#set par(justify: true, leading: 0.62em, first-line-indent: 0pt, spacing: 0.75em)
#show math.equation: set text(font: "Literata Math")
#set math.equation(numbering: none)

// Literata Math draws the italic theta as «ϑ»; use the upright «θ», which
// matches the colour and width of the other Greek letters. Write th, not theta.
#let th = math.upright(sym.theta)

#set heading(numbering: (..n) => "B." + n.pos().map(str).join(".") + ".")
#show heading: it => block(above: 1.5em, below: 0.9em)[
  #set text(size: 11pt, weight: "bold")
  #counter(heading).display(it.numbering) #h(0.4em) #it.body
]

#let parno = counter("par")
#let subno = counter("sub")

#let sect(title) = {
  parno.update(0)
  heading(title)
}

#let mark(extra) = context {
  let s = counter(heading).get().at(0)
  let n = parno.get().at(0)
  let parts = ("B.", str(s), ".", str(n), ".")
  if extra != none { parts += (str(extra), ".") }
  strong(parts.join(""))
}

#let p(body, tag: none) = {
  parno.step()
  subno.update(0)
  block(above: 0.75em, below: 0.75em, width: 100%)[#mark(none) #body#if tag != none [#metadata(none)#tag]]
}

#let pp(body, tag: none) = {
  subno.step()
  context {
    let k = subno.get().at(0)
    pad(left: 1.4em, block(above: 0.5em, below: 0.5em, width: 100%)[
      #mark(k) #body#if tag != none [#metadata(none)#tag]
    ])
  }
}

// Clause reference by label: #rf(<tag>) prints «B.3.12.3».
#let rf(tag) = context {
  let loc = query(tag).first().location()
  let s = counter(heading).at(loc).at(0)
  let n = parno.at(loc).at(0)
  let k = subno.at(loc).at(0)
  let out = "B." + str(s) + "." + str(n)
  if k > 0 { out += "." + str(k) }
  out
}

#let whr(body, lead: none) = {
  if lead != none { block(above: 0.55em, below: 0.35em, lead) }
  pad(left: 1.4em, block(above: 0.25em, below: 0.6em, body))
}

#let eq(body) = block(above: 0.7em, below: 0.7em, width: 100%, align(center, body))

// =============================================================================
//  Text
// =============================================================================

#align(center)[
  #block(width: 85%)[
    #set text(size: 13pt, weight: "bold")
    Appendix B. Rules for compiling the rating list \ (new system)
  ]
]

#v(1.2em)

#sect[General provisions]

#p[The Rating is a system of numerical estimates of the strength of teams and players in competitive «What? Where? When?». It is built from the results of tournaments included in the Rating and published as Releases.]

#p[A Release is the list of teams sorted by team rating in descending order, as of the Thursday of the week in question. A list of players sorted by individual rating in descending order is published together with each Release.]

#p[The identifiers of a team and a player are their identifiers in the rating database (`team_id` and `player_id`). A team keeps its identifier through any change of lineup.]

#p[The Rating includes tournaments marked as rated in the database with a start date on or after #k_start and before #k_to#[;] the upper bound excludes tournaments announced but not yet played. Marathons, asynchronous tournaments and tournaments of unrecognised type are not included.]

#p[The weight $w_t$ of a tournament is a multiplier on the evidence its results carry, set by the format:]
#whr[
  in-person tournament (regular or regional)~– #k_w_in#[;] \
  strictly synchronous tournament~– #k_w_ssync#[;] \
  synchronous tournament~– #k_w_sync.
]

#p[A rated result is a team result marked `inRating` in the database. An unrated result belongs to a composite team with no base lineup. It gives the team no rating update, no rated tournament and no right to appear in a Release, and it does not open a team state. The team still stands in the field as an opponent for all others and enters the pairwise comparison (#rf(<pairwise>)) with its tournament strength, and its players are updated in the question model in the usual way.]

#p[The number of questions $n_q$ in a tournament is the longest mask among its results. If no masks exist, $n_q$ is the sum of question counts over rounds. The team rating does not read the number of questions.]

#p[Events are processed in ascending order of tournament start date, and among equal dates in ascending order of tournament identifier. Processing is done once, in forward chronological order; a published rating value is never revised.]

#p[The rated quantities are:]
#whr[
  the team rating and its usual lineup~– for every team; \
  the player strength, its uncertainty and its volatility~– for every player.
]

#p[A team rating and a player rating change only through the results of a rated tournament played. Inactivity does not change a team rating; it widens the uncertainty of a player estimate but does not lower the estimate.]

#p[A Release includes only the teams and players that played at least one rated tournament in the #k_window days before the Release date. For a team only rated results count. Being left out of a Release does not change the state of a team or a player.]

#sect[Individual rating]

#p[The state of player $i$ is the set of quantities: strength $th_i$, uncertainty $sigma_i^2$ and volatility $v_i$.]

#p(tag: <qmodel>)[The question model is a probabilistic model of a lineup answering a question. Question $q$ has a difficulty $b_q$ and a log discrimination $ln a_q$. The probability that lineup $L$ answers question $q$ is]
#eq[$ P_(L,q) = 1 - exp(-u_q), quad u_q = sum_(i in L) lambda_(i,q), quad lambda_(i,q) = exp(a_q (th_i - b_q)). $]
#whr(lead: [where:])[
  $lambda_(i,q)$~– the intensity of player $i$ on question $q$; \
  $u_q$~– the total intensity of the lineup on question $q$; \
  $a_q$~– the discrimination of question $q$; \
  $b_q$~– the difficulty of question $q$; \
  $L$~– the lineup that played the tournament.
]
#p[The model in #rf(<qmodel>) is the «noisy OR» model: a question counts as answered if at least one player of the lineup answers it, and the players' contributions are independent.]

#p(tag: <intensity>)[The intensity $lambda_(i,q)$ is the rate at which player $i$ finds the answer to question $q$. It depends only on the player's strength and the question's parameters: it equals 1 when the strength equals the difficulty, and every unit of strength above the difficulty multiplies it by $e^(a_q)$, every unit below divides it by the same. A player alone answers with probability $1 - exp(-lambda_(i,q))$, so intensity 1 means a 63% chance and intensity 0.1 about 10%. Intensities of a lineup add because the players search independently, and this sum is what makes the lineup's chance $1 - exp(-u_q)$. Uncertainty and volatility take no part in the answer probability; they govern only how far a player's estimate moves after a tournament (#rf(<update>), #rf(<diffusion>)).]

#p[Saturation is the clamping of $u_q$ to the interval $[exp(-u_"max"), u_"max"]$ before it enters the probability, with $u_"max"$~= #k_u_max. Without saturation a maximally confident prediction against the observed outcome makes the log loss infinite.]

#p(tag: <priors>)[The prior distributions of the question parameters are $ln a_q ~ N(0, s_a^2)$ and $b_q ~ N(overline(b), s_b^2)$, with $s_a$~= #k_s_a, $overline(b)$~= #k_b_bar, $s_b$~= #k_s_b.]

#p[The log likelihood of a tournament is the sum of log probabilities over all questions and all teams of the tournament, multiplied by $w_t$; the prior terms are not multiplied by the weight. In addition:]
#pp[questions marked in the mask as withdrawn or unknown are excluded;]
#pp[results with no mask take no part in the question model.]

#p(tag: <update>)[The update after a tournament is a sequence of three steps.]
#pp(tag: <qfit>)[Question fit. With player strengths fixed, each question $q$ independently takes $a_q = 1$, and the difficulty $b_q$ is set in closed form so that the model predicts exactly the observed answer rate for a lineup of average reach:]
#eq[$ b_q = 1/(|cal(L)_q|) sum_(L in cal(L)_q) ln R_L - ln(-ln(1 - hat(r)_q)), quad R_L = sum_(i in L) exp(th_i), $]
#whr(lead: [where:])[
  $cal(L)_q$~– the lineups that played question $q$;   $R_L$~– the reach of lineup $L$;   $hat(r)_q$~– the share of lineups in $cal(L)_q$ that answered, clamped to no less than 0.02 and no more than 0.98, which keeps a question answered by everyone or by no one on the scale.
]
#pp[Player update. With question parameters fixed, for every player $i$ who played the tournament the gradient $g_i$ and the observed Fisher information $I_i$ of the tournament log likelihood with respect to $th_i$ are computed, and then]
#eq[$ I_"eff" = w_t I_i d_"info", quad pi'_i = 1 \/ sigma_i^2 + max(I_"eff", I_"min"), quad Delta th_i = (w_t d_"info" g_i) / pi'_i, quad sigma_i^2 <- 1 \/ pi'_i. $]
#whr(lead: [where:])[
  $d_"info"$~– the information discount, #k_info_scale#[;] \
  $I_"min"$~– the information floor, #k_info_floor#[;] \
  $sigma_i^2$~– the uncertainty after the increment of #rf(<diffusion>), taken before the update.
]
#p[The strength increment is capped in absolute value at #k_theta_step.]
#pp[Shrinkage to the lineup. After the update every player of the lineup is pulled towards the mean strength of the lineup:]
#eq[$ th_i <- th_i + f dot (1/(|L|) sum_(j in L) th_j - th_i), quad f = #k_shrink. $]

#p[The discount $d_"info"$ exists because the answers of one team are not independent given the player strengths: one tournament carries less evidence than the curvature of the likelihood claims. Its value is set by the honesty requirement on the published uncertainty (#rf(<honest>)).]

#p[Shrinkage to the lineup exists because the «noisy OR» likelihood credits an answer to whoever is already stronger and drives partners who play the same questions thousands of places apart. The value $f = 0$ switches shrinkage off.]

#p(tag: <diffusion>)[Between tournaments the uncertainty of a player estimate grows by $v_i^2 Delta t$, where $Delta t$ is the interval in years and $v_i$ the player's volatility. Volatility is individual and is updated after every tournament with $I_"eff" > I_"min"$ from the standardised surprise]
#eq[$ z_i = (g_i \/ I_i) / sqrt(sigma_i^2 + 1 \/ I_"eff"), quad ln v_i <- ln v_i + eta_v (ln v_* - ln v_i), $]
#whr(lead: [where:])[
  $sigma_i^2$~– the uncertainty before the update; \
  $eta_v$~– the volatility learning rate, #k_v_eta#[;] \
  $v_*$~= $v_"max"$~= #k_v_max if $|z_i| > 1$, and $v_*$~= $v_"min"$~= #k_v_min otherwise.
]
#p[A long, steady history therefore drives the uncertainty to its floor regardless of the calendar, and only a run of results unlike the past opens it again. A new player's volatility is #k_tau_p.]

#p[A newcomer is a player with no rated tournaments played. A newcomer enters the Rating with $th$~= #k_mu_new and $sigma$~= #k_sigma_new. For every player the number of rated tournaments played and the date of the last one are stored.]

#p[The question parameters of a played tournament are stored for reporting and are not used in later updates: a question is never asked again.]

#p[Scale calibration is an operation performed when each Release is compiled. Let]
#eq[$ Delta = 1/(|E|) sum_(i in E) th_i, $]
#whr(lead: [where:])[
  $E$~– the set of players who played at least #k_n_est rated tournaments in the preceding #k_window days; \
  $Delta$~– the scale shift of this Release.
]
#p[$Delta$ is subtracted from every player's strength. The mean strength of established players in every Release is therefore zero. Question difficulties are not corrected, since questions are never asked again. The team rating is not corrected either: it is not a quantity on the $th$ scale, and calibration reaches it only through player strengths, from which the lineup estimate and a new team's first rating are read.]

#p[Player states are stored before the shift is subtracted, and the accumulated shift is subtracted on reading. The published change of a quantity since the previous Release that included it is therefore its own movement, free of scale calibration.]

#p[The individual rating is a direct estimate of the question model. It is published together with its uncertainty; no grade transition rules, thresholds or limits on change apply to it.]

#p(tag: <reserved>)[Reserved. The Rules provide for three extensions of the question model that are implemented and tested but switched off in this edition; none of them lowered the cross-validated log loss of test #rf(<t_ll>). The first is a free number of steps: a question gets a parameter $m_q$ and the answer probability becomes $P_(L,q)^(m_q)$, which penalises lineups of sharply unequal strength relative to even ones; the mean fitted $m_q$ was #k_m_mean and the gain was in-sample only. The second is editor affinity: a player gets a vector $u_i$, an editor a vector $v_e$, and the player's strength in a tournament grows by the mean dot product $u_i dot v_e$ over the tournament's editors; the learned values did not exceed #k_aff_mag on the strength scale, since the Fisher information for $u_i$ is smaller than for $th_i$ and one forward pass is not enough to extract it. The third is the Newton question fit: instead of the closed form of #rf(<qfit>), $(b_q, ln a_q)$ are fitted by damped Newton iterations with step halving on the penalised log likelihood, at most #k_newton iterations, clamped to $a_q in [#k_a_min, #k_a_max]$ and $b_q in [#k_b_min, #k_b_max]$; the priors of #rf(<priors>) serve this fit. It lowers the cross-validated log loss by 0.0009 and the in-sample log loss by 0.018, within the test tolerance, and was replaced by the closed form for simplicity of exposition. All three extensions remain in the implementation behind their switches and are reserved for a future edition of these Rules.]

#sect[Team rating]

#p(tag: <pscale>)[The strength of player $i$ on the team scale is]
#eq[$ r_i = r_0 + s dot (th_i + ln 6 + delta_0), $]
#whr(lead: [where:])[
  $r_0$~– the zero of the team scale, #k_zero#[;] \
  $s$~– the slope of the team scale, #k_scale#[;] \
  $delta_0$~– the lineup estimate correction, #k_roster_bias.
]
#p[The map #rf(<pscale>) is chosen so that six equal players of strength $r$ form a lineup of strength $r$ (#rf(<lineup>)). The zero $r_0$ is free: the Rules never read the absolute level of a rating, and a common shift of every quantity changes no forecast and no place. The slope $s$ is not free: it enters the update step relative to the constant $D$.]

#p(tag: <lineup>)[The lineup estimate $Q(L)$ of lineup $L$ is]
#eq[$ Q(L) = s dot (ln sum_(i in L) exp(r_i \/ s) - ln 6). $]
#p[$Q(L)$ is the strength of a single player who would answer as many questions as the whole lineup together, expressed on the team scale. A short lineup is weaker than a full one, and a strong player in an uneven lineup enters the estimate with more weight than in the arithmetic mean; both follow from the construction, with no separate penalties. The correction $delta_0$ and the zero $r_0$ of #rf(<pscale>) cancel, so only the slope enters the lineup estimate.]

#p[The state of team $T$ is the set of quantities:]
#whr[
  $R_T$~– the team rating on the published scale; \
  $overline(Q)_T$~– the usual lineup of the team: the lineup estimate of the lineups that earned the rating; \
  $n_T$~– the number of rated tournaments of the team; \
  the date of the team's last rated tournament.
]

#p[A new team is one with no state. The state is opened by the first rated result with $R_T = overline(Q)_T = Q(L)$, where $L$ is the lineup in that tournament. An unrated result does not open a team state.]

#p[The field of a tournament is the set of its results that have a non-empty lineup or a previously opened state; a result with neither is skipped. The quantities of #rf(<field>)–#rf(<pairwise>) read player states as they were before the tournament and are computed for the whole field before any new state is written.]

#p(tag: <field>)[The tournament strength of a team is]
#eq[$ F_T = R_T + (Q(L) - overline(Q)_T). $]
#p[A team rating is judged against the team's own usual lineup: guest players raise the tournament strength, absent players lower it, and neither changes $R_T$. A team opening its state enters the field with $F_T = Q(L)$; a result that lists no lineup enters with $F_T = R_T$.]

#p(tag: <pairwise>)[The pairwise comparison is the following procedure. For every pair of teams $A$ and $B$ in the field]
#eq[$ P_(A B) = 1 / (1 + 10^((F_B - F_A) \/ D)), quad S_A = (sum_(B != A) y_(A B) - sum_(B "not tied with" A) P_(A B)) / (N - 1). $]
#whr(lead: [where:])[
  $P_(A B)$~– the expected share of wins of team $A$ over team $B$; \
  $y_(A B)$~– the outcome of the pair: 1 if $A$ placed above $B$, 0 if below; \
  $D$~– the scale constant, #k_D#[;] \
  $N$~– the number of teams in the field; \
  $S_A$~– the surprise of team $A$'s result.
]
#p[A pair of teams that shared a place is skipped in both sums: such a pair says nothing about which of the two is stronger. The score margin does not enter the pairwise comparison.]

#p(tag: <winner>)[Since $y_(A B) - P_(A B) >= 0$ for every pair in which team $A$ placed higher, a team that took first place has $S_A >= 0$. The placement step therefore never lowers the rating of a team that finished first, whether its first place is sole or shared; the floor of #rf(<apply>) keeps that true of the whole update. This property is the winner rule.]

#p(tag: <apply>)[The update after a tournament is the following, performed only on a rated result and in this order:]
#eq[$ R_T <- R_T + K w_t D S_T, quad R_T <- R_T + K_r w_t (Q(L) - R_T), $]
#eq[$ R_T <- max(R_T, R_T^0) "if team" T "finished first", quad overline(Q)_T <- overline(Q)_T + K_q (Q(L) - overline(Q)_T), quad n_T <- n_T + 1, $]
#whr(lead: [where:])[
  $K$~– the rating step, #k_K#[;] \
  $K_r$~– the lineup pull, #k_Kr#[;] \
  $K_q$~– the usual-lineup step, #k_Kq#[;] \
  $R_T^0$~– the rating the team held before the update; \
  $w_t$~– the tournament weight.
]

#p[The second step is the lineup pull, and it is what gives a published team rating a meaning of its own. The placement step alone records who beat whom and nothing about who was at the table, so the rating taken by itself orders a field of teams playing in their base rosters worse than the rating it replaces. The pull moves the rating a fixed share of the way towards what the players who actually turned up are worth, so the evidence of the question model reaches the team number through the only door open to it: a tournament played. Both the pull and the usual-lineup step are performed only when the lineup is non-empty, and $K_q$ is of the same order as $K_r$ so that the lineup offset of #rf(<field>) does not count today's table a second time.]

#p[The third step is the winner floor: a team that took first place, sole or shared, ends the tournament with at least the rating it came with. The pull on its own could lower a team rated above the worth of the lineup it won with; the floor prevents that. At $K_r = 0$ the floor never binds.]
#p[An unrated result stands in the field as an opponent for all others and gives the team no update, no rated tournament and no right to appear in a Release; the date of the last rated tournament advances only on a rated result.]

#p[Inactivity does not change a team rating: nothing happens to a team state between tournaments. A change of lineup does not change a team's rating on the day it is announced either: a team that changes its roster and plays nothing keeps its number. What a changed lineup changes at once is the tournament strength of #rf(<field>); from the next rated tournament on, the rating follows the real lineup through the pull and the usual lineup follows it through $K_q$, the first over roughly three tournaments and the second over roughly ten.]

#p[The constants $D$, $K$, $K_r$ and $K_q$ are set outside the publication loop (#rf(<offline>)).]

#sect[Forecast]

#p[The forecast is an object separate from the rating: an estimate of the strength with which a team will play a given tournament with a given lineup. The forecast takes no part in the rating update and has no effect on it.]

#p(tag: <forecast>)[The forecast strength of a team is the lineup estimate of its lineup, $Q(L)$ (#rf(<lineup>)); with no lineup, its tournament strength $F_T$ (#rf(<field>)).]

#p[The forecast place of a team is its place in the ordering of the field by forecast strength in descending order; first place is the largest forecast strength, and equal values share the mean of their places.]

#p[The forecast does not read the team state at all. This is not a simplification but the outcome of test #rf(<t_sp>): on in-person tournaments of the last two seasons the lineup predicts the result better than the team rating, better than the rating with the lineup offset, and better than the forecast of the old system. Teams are stable; the lineup is not, and the lineup is what answers that question.]

#sect[Tournament difficulty]

#p[The difficulty $D_t$ of a tournament is computed from the fitted parameters of its questions against a fixed reference lineup of #k_ref_size equal players of strength $th^*$:]
#eq[$ p_q = (1 - exp(-6 exp(a_q (th^* - b_q))))^(m_q), quad D_t = 10 (1 - 1/(|Q|) sum_(q in Q) p_q), $]
#whr(lead: [where:])[
  $Q$~– the set of questions of the tournament answered by at least one team; \
  $p_q$~– the probability that the reference lineup answers question $q$; \
  $th^*$~– the reference strength, #k_theta_ref.
]
#p[Difficulty is undefined if no question of the tournament was answered.]

#p[The reference strength $th^*$ is the strength of a balanced team of places #k_band, expressed as #k_ref_size equal players. It matches the band taken as reference in the published TrueDL index, so $D_t$ means what that index promises: ten times the share of the package such a team would not answer, computed from fitted difficulties rather than from a table of ratios between bands. $th^*$ is fixed at launch and is not recomputed.]

#sect[Published quantities]

#p(tag: <display>)[The published rating is an affine map of the internal value:]
#eq[$ R_"team" = A_T + B_T R_T, quad R_"player" = A_P + B_P th_i. $]
#whr(lead: [where:])[
  $A_T$~= #k_A_T, $B_T$~= #k_B_T~– the constants of the team scale; \
  $A_P$~= #k_A_P, $B_P$~= #k_B_P~– the constants of the individual scale.
]

#p[The constants of both scales were set once from the condition that the published values at places #k_pos_hi and #k_pos_lo match the old system's values at the same places: for teams #k_ref_t_50 and #k_ref_t_1000 in the Release nearest #k_anchor_t, for players #k_ref_p_50 and #k_ref_p_1000 in the Release nearest #k_anchor_p. The internal team scale is set by the constants $r_0$ and $s$ of the map #rf(<pscale>); the published map only carries it to the familiar level. The top of the table sits where the internal scale puts it. Once adopted, the constants are not revised.]

#p[A team rating is provisional when the team has played fewer than #k_min_games rated tournaments, and a player rating is provisional when its uncertainty exceeds #k_prov_sigma on the internal scale. Provisional estimates are published with a mark and are left out of the ranked tables.]

#p[Together with a team's published rating the Release shows its number of rated tournaments, its usual lineup, the date of its last rated tournament and the change of rating since the previous Release that included the team. Together with a player's published rating the Release shows the uncertainty of the estimate, the number of rated tournaments in the preceding #k_window days, the date of the last one and the same change. The change is free of scale calibration and is zero on first appearance and in a week without games. The uncertainty of a team rating is not published: Model S2 does not estimate it.]

#p[For every tournament its difficulty is published, and for every team its forecast strength, forecast and actual places, rating before and after the tournament, tournament strength, lineup offset, lineup estimate, surprise of the result and whether the result is rated.]

#sect[Verification]

#p[These Rules were adopted on the results of tests run on the data of the whole covered period. The test results:]

#block(above: 0.8em, below: 0.9em, width: 100%)[
  #set text(size: 9.5pt)
  #set par(justify: false, leading: 0.55em)
  #table(
    columns: (1.35fr, 1.25fr, 1.5fr, auto),
    inset: (x: 5pt, y: 4.5pt),
    stroke: 0.4pt,
    align: (left, left, left, center),
    table.header(
      [*Test*], [*Requirement*], [*Result*], [*Outcome*],
    ),
    [Cross-validated log loss on in-person rated tournaments],
    [lower than the nested baseline model],
    [#k_ll_cross over all seasons, #k_ll_2526 in season 2025/26],
    [passed],

    [Spearman rank correlation of the forecast on in-person tournaments of the last two seasons],
    [within 0.02 of the best external system],
    [#k_sp_new against #k_sp_old for the old system; on synchronous tournaments #k_sp_new_s against #k_sp_old_s],
    [passed],

    [Overlap of the top #k_ov_n of the individual list with external systems],
    [at least #k_ov_need of #k_ov_n],
    [#k_ov_quest of #k_ov_n with chgk.quest, #k_ov_fun of #k_ov_n with rating.chgk.fun],
    [failed],

    [Inflation: published value of the #{k_pos_hi}th team on September Releases #k_infl_from–#k_infl_to],
    [within the band #k_infl_band],
    [largest deviation #k_infl over #k_infl_n Releases],
    [passed],

    [Transfer invariance: swapping the strongest players between two teams],
    [no team rating changes, forecasts change],
    [0 teams changed, largest difference exactly 0; the forecast changed for #k_swap_n results],
    [passed],

    [Winner rule],
    [the first-placed team's rating after the tournament is not below its rating before],
    [0 violations],
    [passed],
  )
]

#p(tag: <t_sp>)[The forecast rank correlation test is passed with a margin, not within tolerance: the lineup forecast beats the old system's forecast by #k_sp_new against #k_sp_old on in-person tournaments and by #k_sp_new_s against #k_sp_old_s on synchronous ones. On the same tournaments the team rating alone gives 0.66 and the rating with the lineup offset 0.83, so the lineup predicts the result better than the accumulated rating.]

#p(tag: <t_ll>)[The log loss test decides the composition of the model: an extension stays in the Rules only if it lowers this figure. For comparison, the external system chgk.quest publishes #k_ll_quest, measured in-sample.]

#p[#k_infl_skip is excluded from the September Releases: fewer than two hundred rated teams played in the year before it, which is a gap in the data, not inflation.]

#p[The top-#k_ov_n overlap test is failed. The threshold of #k_ov_need of #k_ov_n is above what mature external systems reach with each other: chgk.quest and rating.chgk.fun agree on #k_ov_mutual of #k_ov_n. The rank correlation of the strength of players with at least #k_corr_games tournaments is #k_corr_quest with chgk.quest and #k_corr_fun with rating.chgk.fun.]

#p(tag: <offline>)[Outside the publication loop the following are set: the correction $delta_0$~– as the mean difference between a team's realised strength and the uncorrected sum of its players' strengths over full six-player lineups on in-person tournaments of 2015–2024; the constant $D$~– as the maximum-likelihood value for the pairwise outcomes, the likelihood being flat from 900 to 2400; the step $K$~– as the maximum of the mean Spearman rank correlation of the forecast on in-person tournaments from #k_tune_from to #k_tune_to over the grid 0.2, 0.3, 0.4 and 0.6; the pull $K_r$ and the usual-lineup step $K_q$~– as the maximum, over the same tournaments, of the mean quality of the order the rating alone imposes on a field of teams playing in their base rosters, each tournament weighted by the strength of its ten strongest lineups#[;] the constants of the individual scale~– from the 2019 Release (#rf(<display>)).]

#p(tag: <honest>)[The honesty requirement on uncertainty is the condition that the standardised residuals $z_i$ have unit variance on in-person tournaments from 2018 onwards. This requirement sets the discount $d_"info"$. It does not apply to the team rating: Model S2 neither estimates nor publishes an uncertainty.]

#sect[Transitional provisions]

#p[The initial recomputation is the processing of all rated tournaments from #k_start under the rules of sections B.2 and B.3 in chronological order.]

#p[In the first season there is no history: every player enters with the newcomer prior at once, the question difficulties of the strong 2013 field come out too low, and its players too high. The initial recomputation is therefore run in #k_passes passes, and the outputs are written only on the last.]
#pp[The first pass follows the general rules. On first reaching a tournament dated on or after #k_warm_until it stores the strength of every player who has appeared so far.]
#pp[Every later pass starts again from #k_start: stored players enter with their stored strength, uncertainty #k_warm_sigma and volatility #k_tau_p, and all others count as never seen and enter with the newcomer prior at their first tournament. Team states are rebuilt from scratch in every pass.]

#p[Since the store is taken on #k_warm_until and no player who debuted after that date gets an initial estimate, no value after that date rests on its own future.]

#p[After the initial recomputation publication proceeds in a single forward pass. Full recomputations happen only outside the publication loop, to tune constants and for the initial recomputation. A published Release is never rewritten.]

#p[The quantities marked tunable in these Rules were set once and frozen on the date the Rules came into force. Any later change to them is a change to these Rules.]

#v(1.6em)
#align(center)[\* \* \*]
