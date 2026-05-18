# Plan d'implementation du moteur de decision

Ce document decrit le plan technique pour transformer le scanner actuel en moteur de decision capable de produire:

- un signal d'entree;
- un prix d'entree exploitable;
- un stop de perte base principalement sur le sous-jacent;
- des objectifs de sortie en gain;
- une duree de detention cible sur plusieurs semaines;
- un score de confiance;
- un affichage Ratatui oriente decision.

Le but n'est pas de produire une recommandation certaine, mais un cadre quantitatif coherent, testable et explicable. Chaque signal doit pouvoir afficher son calcul.

## Etat actuel

| Bloc | Etat | Fichiers actuels | Commentaire |
| --- | --- | --- | --- |
| Recuperation spot | Fait | `src/api/fetch.rs`, `src/models/warrant.rs` | Spot, devise et variation via Yahoo. |
| Decouverte Euronext | Fait | `src/api/discover.rs` | Source principale des produits. |
| Fallback Boursorama discovery | Fait | `src/api/boursorama_discover.rs` | Utile pour Hermes quand Euronext ne renvoie rien. |
| Enrichissement Boursorama quote | Fait | `src/api/boursorama.rs` | Bid, ask, tailles, mid, volume. |
| Parsing parite/strike/barriere | Partiel solide | `src/models/structured.rs`, `src/pricing.rs` | Doit encore recevoir des tests de non-regression plus larges. |
| FX EUR/USD | Fait | `src/api/fx.rs`, `src/models/fx.rs` | Conversion utilisee dans les calculs de valeur. |
| Nettoyage bid/ask | Fait, a etendre | `src/indicators/warrant.rs` | Rejette bid/ask invalides et spreads extremes. |
| Edge net | Fait | `src/indicators/warrant.rs` | `spread_adjusted_gap_pct = gap - spread` pour sous-evalues. |
| IV Black-Scholes | Partiel | `src/indicators/options.rs` | Present, mais non exploite pour la decision globale. |
| Volatility smile | Partiel | `src/indicators/warrant.rs` | Median IV par pairs, pas encore une courbe robuste. |
| TUI opportunites | Fait, a refondre | `src/display/opportunities.rs` | Affichage actuel oriente ranking, pas encore plan de trade. |
| Moteur stop/target | Fait | `src/decision/` | Coeur du plan branche dans `opportunities`. |
| Position sizing | Fait | `src/decision/position.rs` | Taille selon risque compte, stop et plafond de notionnel. |
| Tests decision | Fait, a etendre | `src/decision/*` + tests unitaires | Couvre conversion, stop, target, R/R, data quality. |
| Bougies cachees | Fait | `src/api/candles.rs`, `src/api/candle_cache.rs` | Cache local `data/cache/candles`, refresh forcable. |
| ATR/support/resistance | Fait | `src/decision/market_indicators.rs` | Alimente les stops et targets. |

## Architecture cible

Ajouter un module `src/decision/`:

```text
src/decision/
  mod.rs
  config.rs
  models.rs
  pricing_projection.rs
  risk.rs
  targets.rs
  scoring.rs
  trade_plan.rs
  position.rs
  time.rs
  display.rs
```

Modifier ensuite:

```text
src/main.rs
src/indicators/warrant.rs
src/display/opportunities.rs
src/display/headless.rs
src/config.rs
manage.sh
.env.example
```

Flux cible:

```text
Spot + Produits + Quotes + FX
        |
        v
rank_relative_value()
        |
        v
OpportunitySignal
        |
        v
build_trade_plan(signal, market, config)
        |
        v
DecisionSignal { opportunity, trade_plan }
        |
        v
TUI / CSV / headless
```

## Structures de donnees

### `DecisionConfig`

Fichier: `src/decision/config.rs`

```rust
pub struct DecisionConfig {
    pub min_entry_edge_pct: f64,
    pub min_confidence_score: f64,
    pub max_spread_pct: f64,
    pub min_data_quality_score: f64,
    pub min_liquidity_score: f64,
    pub min_peer_count: usize,
    pub min_reward_risk: f64,
    pub max_loss_pct_per_trade: f64,
    pub account_risk_pct: f64,
    pub max_position_notional_pct: f64,
    pub default_holding_days: u32,
    pub max_holding_days: u32,
    pub atr_period_days: u32,
    pub stop_atr_multiple: f64,
    pub target_1_r_multiple: f64,
    pub target_2_r_multiple: f64,
    pub min_barrier_distance_pct: f64,
    pub barrier_stop_buffer_pct: f64,
    pub max_theta_to_horizon_pct: f64,
    pub min_maturity_days: u32,
}
```

Entrave `.env`:

```text
DECISION_MIN_ENTRY_EDGE_PCT=1.0
DECISION_MIN_CONFIDENCE_SCORE=75
DECISION_MAX_SPREAD_PCT=5
DECISION_MIN_DATA_QUALITY_SCORE=80
DECISION_MIN_LIQUIDITY_SCORE=50
DECISION_MIN_PEER_COUNT=5
DECISION_MIN_REWARD_RISK=1.5
DECISION_MAX_LOSS_PCT_PER_TRADE=25
DECISION_ACCOUNT_RISK_PCT=1
DECISION_MAX_POSITION_NOTIONAL_PCT=10
DECISION_DEFAULT_HOLDING_DAYS=21
DECISION_MAX_HOLDING_DAYS=35
DECISION_ATR_PERIOD_DAYS=14
DECISION_STOP_ATR_MULTIPLE=1.5
DECISION_TARGET_1_R_MULTIPLE=1.5
DECISION_TARGET_2_R_MULTIPLE=2.5
DECISION_MIN_BARRIER_DISTANCE_PCT=8
DECISION_BARRIER_STOP_BUFFER_PCT=2
DECISION_MAX_THETA_TO_HORIZON_PCT=15
DECISION_MIN_MATURITY_DAYS=21
```

### `MarketContext`

Fichier: `src/decision/models.rs`

```rust
pub struct MarketContext {
    pub underlying_ticker: String,
    pub spot: f64,
    pub spot_currency: String,
    pub change_pct: f64,
    pub fx_rates: FxRateBook,
    pub realized_volatility_20d: Option<f64>,
    pub atr_14d: Option<f64>,
    pub support_1: Option<f64>,
    pub support_2: Option<f64>,
    pub resistance_1: Option<f64>,
    pub resistance_2: Option<f64>,
    pub risk_free_rate: f64,
    pub dividend_yield: f64,
}
```

Input:

- spot Yahoo;
- FX book existant;
- ATR/support/resistance calcules a partir d'historique quand disponible.

Output:

- contexte unique passe au moteur de decision.

### `DecisionSignal`

Fichier: `src/decision/models.rs`

```rust
pub struct DecisionSignal {
    pub opportunity: OpportunitySignal,
    pub trade_plan: TradePlan,
    pub decision: DecisionAction,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
}
```

### `DecisionAction`

```rust
pub enum DecisionAction {
    BuyCandidate,
    Watch,
    Avoid,
    ExitLoss,
    TakeProfit,
    Hold,
}
```

Signification:

- `BuyCandidate`: les conditions d'entree sont remplies.
- `Watch`: signal interessant, mais une condition importante manque.
- `Avoid`: data ou risque trop mauvais.
- `ExitLoss`: utilise plus tard si une position ouverte est fournie.
- `TakeProfit`: utilise plus tard si une position ouverte est fournie.
- `Hold`: position ouverte encore valide.

### `TradePlan`

Fichier: `src/decision/models.rs`

```rust
pub struct TradePlan {
    pub entry_price: f64,
    pub entry_price_source: String,
    pub entry_underlying_price: f64,
    pub entry_edge_pct: f64,
    pub breakeven_move_pct: Option<f64>,
    pub stop: StopPlan,
    pub target_1: TargetPlan,
    pub target_2: TargetPlan,
    pub reward_risk_1: Option<f64>,
    pub reward_risk_2: Option<f64>,
    pub horizon: HorizonPlan,
    pub risk: RiskPlan,
    pub position: PositionPlan,
    pub confidence_score: f64,
}
```

### `StopPlan`

```rust
pub struct StopPlan {
    pub underlying_stop_price: f64,
    pub estimated_product_stop_price: f64,
    pub loss_pct: f64,
    pub distance_to_stop_pct: f64,
    pub stop_reason: StopReason,
}

pub enum StopReason {
    Atr,
    TechnicalSupportResistance,
    BarrierBuffer,
    MaxLoss,
    FallbackPercent,
}
```

### `TargetPlan`

```rust
pub struct TargetPlan {
    pub underlying_target_price: f64,
    pub estimated_product_target_price: f64,
    pub gain_pct: f64,
    pub distance_to_target_pct: f64,
    pub target_reason: TargetReason,
}

pub enum TargetReason {
    RiskMultiple,
    TechnicalSupportResistance,
    IntrinsicRepricing,
    FallbackPercent,
}
```

### `HorizonPlan`

```rust
pub struct HorizonPlan {
    pub holding_days: u32,
    pub max_holding_days: u32,
    pub maturity_days: Option<i64>,
    pub theta_daily_pct: Option<f64>,
    pub theta_to_horizon_pct: Option<f64>,
    pub time_risk_score: f64,
}
```

### `RiskPlan`

```rust
pub struct RiskPlan {
    pub barrier_distance_pct: Option<f64>,
    pub barrier_risk_score: Option<f64>,
    pub spread_pct: Option<f64>,
    pub spread_cost_underlying_pct: Option<f64>,
    pub data_quality_score: f64,
    pub liquidity_score: f64,
    pub max_loss_pct: f64,
}
```

### `PositionPlan`

```rust
pub struct PositionPlan {
    pub account_risk_pct: f64,
    pub suggested_notional_pct: f64,
    pub max_notional_pct: f64,
    pub estimated_account_loss_pct: f64,
    pub products_per_1000_account: Option<f64>,
}
```

## Fonctions a implementer

### `build_trade_plan`

Fichier: `src/decision/trade_plan.rs`

```rust
pub fn build_trade_plan(
    signal: &OpportunitySignal,
    market: &MarketContext,
    config: &DecisionConfig,
) -> DecisionSignal
```

Input:

- `OpportunitySignal`: signal deja calcule par le ranking relatif;
- `MarketContext`: spot, FX, volatilite realisee, ATR, niveaux techniques;
- `DecisionConfig`: seuils de decision.

Output:

- `DecisionSignal`: opportunite + plan entree/stop/targets + decision.

Logique:

1. Determiner le prix d'entree:
   - priorite `ask_price` si achat long;
   - fallback interdit si `execution_status` non executable;
   - si pas d'ask valide: `Avoid`.
2. Calculer `breakeven_move_pct`.
3. Construire le stop sous-jacent.
4. Projeter le prix produit au stop.
5. Construire deux targets.
6. Projeter les prix produit aux targets.
7. Calculer R/R.
8. Calculer les scores de risque, temps et confiance.
9. Produire `BuyCandidate`, `Watch` ou `Avoid`.

### `entry_price_for_long`

Fichier: `src/decision/trade_plan.rs`

```rust
pub fn entry_price_for_long(signal: &OpportunitySignal) -> Option<f64>
```

Input:

- `OpportunitySignal`.

Output:

- `Some(ask)` si le carnet est executable;
- `None` sinon.

Logique:

```text
si ask_price > 0 et bid_price > 0 et ask >= bid et spread_pct <= max_spread:
    entry = ask
sinon:
    None
```

Note:

- ne jamais utiliser `last_unverified` pour une decision d'entree.

### `compute_breakeven_move_pct`

Fichier: `src/decision/risk.rs`

```rust
pub fn compute_breakeven_move_pct(spread_pct: Option<f64>, effective_gearing: Option<f64>) -> Option<f64>
```

Formule:

```text
breakeven_move_pct = spread_pct / effective_gearing
```

Input:

- spread produit en pourcentage;
- levier effectif.

Output:

- mouvement minimal du sous-jacent necessaire pour absorber le spread.

Exemple:

```text
spread 8.85%, levier 6.67 -> 1.33%
```

### `estimate_product_price_at_underlying`

Fichier: `src/decision/pricing_projection.rs`

```rust
pub fn estimate_product_price_at_underlying(
    signal: &OpportunitySignal,
    underlying_price: f64,
    market: &MarketContext,
) -> Option<f64>
```

Input:

- signal;
- prix futur du sous-jacent;
- contexte FX.

Output:

- prix theorique estime du produit.

Logique pour financement/turbo/mini-future:

```text
call = max(0, underlying - reference) / parity * fx
put  = max(0, reference - underlying) / parity * fx
```

Ou:

- `reference = strike` ou niveau de financement selon `pricing_model`;
- `parity = warrants_per_underlying`;
- `fx = reference_currency -> price_currency`.

Logique pour warrant vanilla:

```text
prix = BlackScholes(S, K, T, r, q, IV) / parity * fx
```

Fallback:

- si IV indisponible: utiliser intrinsic only + prime actuelle conservee;
- afficher warning `iv_missing_projection_degraded`.

### `compute_underlying_stop`

Fichier: `src/decision/risk.rs`

```rust
pub fn compute_underlying_stop(
    signal: &OpportunitySignal,
    market: &MarketContext,
    config: &DecisionConfig,
) -> StopCandidate
```

Output:

```rust
pub struct StopCandidate {
    pub price: f64,
    pub reason: StopReason,
}
```

Logique:

Pour un call:

1. si `support_1` existe et `support_1 < spot`: utiliser support;
2. sinon si ATR existe: `spot - ATR * stop_atr_multiple`;
3. sinon fallback: `spot * (1 - 0.03)`.

Pour un put:

1. si `resistance_1` existe et `resistance_1 > spot`: utiliser resistance;
2. sinon si ATR existe: `spot + ATR * stop_atr_multiple`;
3. sinon fallback: `spot * (1 + 0.03)`.

Regle barriere:

- call: le stop doit rester au-dessus de la barriere avec buffer;
- put: le stop doit rester sous la barriere avec buffer;
- si impossible, decision degradee en `Avoid`.

### `apply_max_loss_cap`

Fichier: `src/decision/risk.rs`

```rust
pub fn apply_max_loss_cap(
    signal: &OpportunitySignal,
    stop: StopCandidate,
    market: &MarketContext,
    config: &DecisionConfig,
) -> StopCandidate
```

But:

- eviter un stop produit qui implique une perte superieure a `DECISION_MAX_LOSS_PCT_PER_TRADE`.

Logique:

1. projeter le prix produit au stop;
2. calculer `loss_pct`;
3. si perte trop grande, rapprocher le stop du sous-jacent;
4. recalculer;
5. si le stop devient trop serre ou incoherent, decision `Watch` ou `Avoid`.

### `compute_targets`

Fichier: `src/decision/targets.rs`

```rust
pub fn compute_targets(
    signal: &OpportunitySignal,
    stop: &StopPlan,
    market: &MarketContext,
    config: &DecisionConfig,
) -> (TargetPlan, TargetPlan)
```

Logique par defaut:

```text
risk_underlying = abs(spot - stop_underlying)
target_1_distance = risk_underlying * target_1_r_multiple
target_2_distance = risk_underlying * target_2_r_multiple
```

Pour call:

```text
target_1_underlying = spot + target_1_distance
target_2_underlying = spot + target_2_distance
```

Pour put:

```text
target_1_underlying = spot - target_1_distance
target_2_underlying = spot - target_2_distance
```

Si support/resistance technique disponible:

- call: preferer resistances proches coherentes;
- put: preferer supports proches coherents.

### `compute_reward_risk`

Fichier: `src/decision/targets.rs`

```rust
pub fn compute_reward_risk(gain_pct: f64, loss_pct: f64) -> Option<f64>
```

Formule:

```text
reward_risk = gain_pct / abs(loss_pct)
```

Retour:

- `None` si perte nulle ou incoherente.

### `compute_time_risk`

Fichier: `src/decision/risk.rs`

```rust
pub fn compute_time_risk(
    signal: &OpportunitySignal,
    config: &DecisionConfig,
) -> HorizonPlan
```

Pour warrants:

```text
theta_to_horizon_pct = theta_daily_pct * holding_days
```

Pour turbos/mini-futures:

- `theta_daily_pct = None`;
- `time_risk_score` depend de:
  - maturite restante;
  - open-end ou date fixe;
  - proximite barriere;
  - niveau de financement.

Regles:

- si maturite datee et `maturity_days < DECISION_MIN_MATURITY_DAYS`: `Avoid`;
- si theta horizon depasse `DECISION_MAX_THETA_TO_HORIZON_PCT`: `Watch` ou `Avoid`.

Etat implemente:

- `src/decision/time.rs` calcule un `HorizonPlan`;
- les turbos/mini-futures n'ont pas de theta optionnel;
- les warrants avec IV utilisent le theta Black-Scholes;
- `theta_to_horizon_pct > seuil` degrade en `Watch`;
- `theta_to_horizon_pct > 2 * seuil` degrade en `Avoid`;
- une maturite inferieure a `DECISION_MIN_MATURITY_DAYS` degrade en `Avoid`.

### `compute_position_plan`

Fichier: `src/decision/position.rs`

```rust
pub fn compute_position_plan(
    entry_price: f64,
    stop_loss_pct: f64,
    config: &DecisionConfig,
) -> Option<PositionPlan>
```

Formules:

```text
suggested_notional_pct = min(
  DECISION_ACCOUNT_RISK_PCT / stop_loss_pct * 100,
  DECISION_MAX_POSITION_NOTIONAL_PCT
)

estimated_account_loss_pct = suggested_notional_pct * stop_loss_pct / 100
products_per_1000_account = (1000 * suggested_notional_pct / 100) / entry_price
```

Exemple:

```text
risque compte 1%, stop produit 20%, entree 2 EUR
taille = 1 / 20 * 100 = 5% du capital
perte compte estimee = 5 * 20 / 100 = 1%
produits / 1000 = 1000 * 5% / 2 = 25 produits
```

### `compute_barrier_risk_score`

Fichier: `src/decision/risk.rs`

```rust
pub fn compute_barrier_risk_score(
    barrier_distance_pct: Option<f64>,
    effective_gearing: Option<f64>,
) -> Option<f64>
```

Formule:

```text
barrier_risk_score = effective_gearing / barrier_distance_pct
```

Interpretation:

- score bas: risque plus confortable;
- score haut: levier fort et barriere proche.

### `compute_confidence_score`

Fichier: `src/decision/scoring.rs`

```rust
pub fn compute_confidence_score(
    signal: &OpportunitySignal,
    plan: &PartialTradePlan,
    config: &DecisionConfig,
) -> f64
```

Formule cible:

```text
confidence =
  data_quality_score * 0.25
+ liquidity_score * 0.15
+ spread_score * 0.15
+ peer_score * 0.10
+ edge_score * 0.15
+ reward_risk_score * 0.10
+ barrier_score * 0.05
+ time_score * 0.05
```

Scores intermediaires:

```text
spread_score = 100 - clamp(spread_pct / max_spread_pct * 100, 0, 100)
peer_score = clamp(peer_count / 10 * 100, 0, 100)
edge_score = clamp(edge_net_pct / 5 * 100, 0, 100)
reward_risk_score = clamp(reward_risk_2 / min_reward_risk * 100, 0, 100)
```

### `decide_action`

Fichier: `src/decision/scoring.rs`

```rust
pub fn decide_action(
    signal: &OpportunitySignal,
    plan: &TradePlan,
    config: &DecisionConfig,
) -> (DecisionAction, Vec<String>, Vec<String>)
```

Regles `Avoid`:

```text
data_quality_score < min_data_quality_score
liquidity_score < min_liquidity_score
spread_pct > max_spread_pct
bid/ask non executable
peer_count < min_peer_count
barrier_distance_pct < min_barrier_distance_pct
maturity_days < min_maturity_days
stop incoherent
target incoherent
```

Regles `BuyCandidate`:

```text
edge_net_pct >= min_entry_edge_pct
confidence_score >= min_confidence_score
reward_risk_2 >= min_reward_risk
data executable
barriere acceptable
time risk acceptable
```

Sinon:

```text
Watch
```

## Indicateurs a afficher

### Indicateurs principaux

| Colonne | Source | Formule | Interpretation |
| --- | --- | --- | --- |
| `Decision` | `DecisionAction` | Regles de scoring | BUY, WATCH, AVOID. |
| `Conf` | `confidence_score` | Score pondere | Qualite globale du signal. |
| `Edge` | `spread_adjusted_gap_pct` | Gap - spread | Signal relatif net. |
| `R/R` | `reward_risk_2` | Gain cible / perte stop | Qualite du plan de sortie. |
| `Stop%` | `stop.loss_pct` | Prix entree vs prix stop | Perte estimee. |
| `T1%` | `target_1.gain_pct` | Target 1 vs entree | Premier objectif. |
| `T2%` | `target_2.gain_pct` | Target 2 vs entree | Deuxieme objectif. |
| `BE%` | `breakeven_move_pct` | Spread / levier | Mouvement minimal du sous-jacent. |
| `Spr` | `spread_pct` | `(ask-bid)/mid` | Cout immediat. |
| `DQ` | `data_quality_score` | Nettoyage data | Fiabilite du prix. |
| `Liq` | `liquidity_score` | Taille/volume/spread | Sortie possible. |
| `Bar%` | `barrier_distance_pct` | Distance barriere | Risque knockout. |
| `Lev` | `effective_gearing` | Produit / sous-jacent | Sensibilite. |
| `IVd` | `smile_gap_vol_points` | IV - smile | Seulement warrants. |

### Colonnes TUI cible

Remplacer la table principale par:

```text
Decision Conf Edge R/R Stop% T1% T2% BE% Spr DQ Liq Symbol Side Mny Mat Price Bar% Lev IVd Parity
```

Tri par defaut:

```text
BuyCandidate d'abord
puis confidence_score desc
puis edge_net desc
puis reward_risk_2 desc
```

Touches TUI:

```text
a: all
c: calls
p: puts
u: sous-evalues
o: sur-evalues
b: afficher seulement BUY
w: afficher WATCH
x: afficher AVOID
1: panneau resume
2: panneau calculs
3: panneau risque
4: panneau raw data
Enter/Espace: ouvrir URL Boursorama
q/Esc: quitter
```

### Details TUI cible

Panneau `Resume`:

```text
Decision BUY_CANDIDATE  Conf 82  Edge +2.4%  R/R 2.1
Entry 5.832 EUR via ask  Stop 4.95 EUR (-15.1%)  T1 6.70 (+14.9%)  T2 7.42 (+27.2%)
Sous-jacent spot 300.23 USD  Stop 293.50  T1 314.00  T2 327.50
```

Panneau `Calculs`:

```text
Edge: gap +2.50% - spread 0.10% = +2.40%
Breakeven: spread 0.10% / levier 4.43 = 0.02% sous-jacent
Stop projection: put = (reference - stop_underlying) / parity * fx
Reward/Risk: target2_gain 27.2 / stop_loss 15.1 = 1.80
```

Panneau `Risque`:

```text
Data: executable_bid_ask, DQ 95, Liq 70, bidSize 50000, askSize 50000
Barrier: distance 16.9%, barrier risk 0.26
Time: open-end, horizon 21 jours, theta -
Warnings: volume 0, IV missing
```

Panneau `Raw data`:

```text
URL, ISIN, emetteur, type produit, strike, strike2, parite, bid, ask, high, low, previous close
```

## CSV cible

Ajouter les colonnes:

```text
decision
confidence_score
entry_price
entry_price_source
underlying_entry_price
underlying_stop_price
product_stop_price
stop_loss_pct
target_1_underlying_price
target_1_product_price
target_1_gain_pct
target_2_underlying_price
target_2_product_price
target_2_gain_pct
reward_risk_1
reward_risk_2
breakeven_move_pct
barrier_risk_score
time_risk_score
theta_to_horizon_pct
max_holding_days
decision_reasons
decision_warnings
```

## Tests unitaires requis

### Projection prix produit

Fichier: `src/decision/pricing_projection.rs`

Cas:

1. Turbo call EUR sous-jacent EUR:
   - spot cible 1600;
   - reference 1500;
   - parite 100;
   - prix attendu 1.0 EUR.

2. Turbo put EUR sous-jacent EUR:
   - spot cible 1575.50;
   - reference 1800;
   - parite 200;
   - prix attendu 1.1225 EUR.

3. Turbo put USD cote EUR:
   - reference 369.61 USD;
   - spot 300.23 USD;
   - parite 10;
   - FX USD->EUR 0.8602;
   - prix attendu environ 5.968 EUR.

4. Warrant call USD cote EUR:
   - convertir correctement le prix via FX;
   - ne jamais additionner EUR et USD dans la meme formule.

### Stop

Fichier: `src/decision/risk.rs`

Cas:

1. Call avec support valide:
   - stop = support.

2. Put avec resistance valide:
   - stop = resistance.

3. Pas de niveau technique:
   - stop = ATR.

4. Pas d'ATR:
   - stop fallback 3%.

5. Stop traverse barriere:
   - `Avoid` ou stop ajuste avec buffer.

### Targets

Fichier: `src/decision/targets.rs`

Cas:

1. Call:
   - target au-dessus du spot.

2. Put:
   - target sous le spot.

3. Target 2 produit gain positif.

4. Reward/risk calcule correctement.

### Decision

Fichier: `src/decision/scoring.rs`

Cas:

1. Signal propre:
   - `BuyCandidate`.

2. Spread trop large:
   - `Avoid`.

3. Data quality trop basse:
   - `Avoid`.

4. Reward/risk trop faible:
   - `Watch`.

5. Barrier distance trop faible:
   - `Avoid`.

6. Maturite trop courte:
   - `Avoid`.

### TUI formatting

Fichier: `src/display/opportunities.rs`

Cas:

1. Pas de panic si IV absente.
2. Pas de panic si stop/target absent.
3. Les colonnes tiennent sur largeur standard.
4. Details affichent les calculs de stop et target.
5. URL toujours visible et Enter/Espace continue de fonctionner.

## Ordre d'implementation recommande

| Etape | Statut | Objectif | Fichiers |
| --- | --- | --- | --- |
| 1 | Fait | Ajouter `DecisionConfig` et variables `.env` | `src/decision/config.rs`, `.env.example` |
| 2 | Fait | Ajouter structures `TradePlan`, `DecisionSignal` | `src/decision/models.rs` |
| 3 | Fait | Implementer projection prix produit | `src/decision/pricing_projection.rs` |
| 4 | Fait | Ajouter tests projection FX/parite | `src/decision/pricing_projection.rs` |
| 5 | Fait | Implementer breakeven, barrier risk, stop | `src/decision/risk.rs` |
| 6 | Fait | Implementer targets et R/R | `src/decision/targets.rs` |
| 7 | Fait | Implementer confidence et decision | `src/decision/scoring.rs` |
| 8 | Fait | Brancher `build_trade_plan` dans `opportunities` | `src/main.rs` |
| 9 | Fait | Ajouter CSV enrichi | `src/main.rs` |
| 10 | Fait | Refondre TUI avec Decision/Conf/RR/Stop/Targets | `src/display/opportunities.rs` |
| 11 | Fait | Ajouter filtres TUI BUY/WATCH/AVOID | `src/display/opportunities.rs` |
| 12 | Partiel | Ajouter tests TUI/render minimal | `src/display/opportunities.rs` compile avec le modele decision, tests render dedies a ajouter |
| 13 | Fait | Integrer les frais broker dans les calculs nets | `src/decision/fees.rs`, `src/decision/trade_plan.rs`, `.env.example`, `manage.sh` |
| 14 | Fait | Exporter et afficher les frais dans CSV/Ratatui | `src/main.rs`, `src/display/opportunities.rs` |
| 13 | Fait | Ajouter commande manage.sh pour tests decision | `manage.sh test-unit` |
| 14 | Fait | Documenter interpretation utilisateur | `docs/technical-and-logic.md`, README a enrichir plus tard |
| 15 | Fait | Ajouter sizing position | `src/decision/position.rs`, TUI, CSV |
| 16 | Fait | Ajouter theta/horizon | `src/decision/time.rs`, `src/indicators/options.rs`, scoring |

## Regles de securite financiere dans le code

Ces regles doivent etre codees comme garde-fous, pas seulement documentees.

1. Pas d'entree si le prix est `last_unverified`.
2. Pas d'entree si `bid <= 0` ou `ask <= 0`.
3. Pas d'entree si `ask < bid`.
4. Pas d'entree si le spread depasse `DECISION_MAX_SPREAD_PCT`.
5. Pas d'entree si la devise du prix et la devise du payoff ne sont pas converties explicitement.
6. Pas d'entree si la parite est absente pour un produit a payoff par sous-jacent.
7. Pas d'entree si le stop produit ne peut pas etre estime.
8. Pas d'entree si le R/R est inferieur au seuil.
9. Pas d'entree si la barriere est trop proche.
10. Pas d'entree si la maturite est incompatible avec l'horizon.

## Definition d'un signal exploitable

Un signal est exploitable seulement si:

```text
decision == BuyCandidate
entry_price vient de ask executable
stop estime existe
target_2 estime existe
reward_risk_2 >= DECISION_MIN_REWARD_RISK
confidence_score >= DECISION_MIN_CONFIDENCE_SCORE
```

Tout autre signal doit etre visible, mais marque `Watch` ou `Avoid`.

## Exemple de sortie cible

```text
Decision BUY  Conf 84  Edge +2.4%  R/R 1.9  Stop -13.0%  T1 +15.0%  T2 +25.0%
Entry 5.832 EUR ask  Spot 300.23 USD
Stop sous-jacent 293.50 USD -> produit 5.07 EUR
Target1 313.80 USD -> produit 6.71 EUR
Target2 326.90 USD -> produit 7.29 EUR
Breakeven 0.02% sous-jacent  Spread 0.10%  DQ 95  Liq 70
Warnings: volume_zero; iv_missing_not_needed_for_turbo
```

## Critere de fin

L'implementation est consideree terminee quand:

1. `./manage.sh test` passe.
2. Les tests unitaires `decision` passent.
3. `./manage.sh opportunities hermes RMS.PA 500 all` affiche Decision/Conf/RR/Stop/Targets.
4. `./manage.sh opportunities apple AAPL 500 all` affiche les memes colonnes.
5. `BROKER_FEE_PROFILE=trade_republic ./manage.sh opportunities apple AAPL 500 all` affiche `Fee`, une ligne `Fees` et un `R/R` net de frais.
5. Enter/Espace ouvre toujours Boursorama et affiche aussi les calculs.
6. Aucun produit avec bid/ask invalide ne peut etre `BuyCandidate`.
7. Un produit sans plan de stop/target est au maximum `Watch`.
8. Le CSV contient les colonnes de decision et de plan de sortie.
