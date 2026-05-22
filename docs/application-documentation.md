# Documentation de l'application Warrant Fetcher

Cette documentation decrit l'etat actuel de l'application: ce qu'elle fait, comment la lancer, comment lire les resultats, quelles donnees sont utilisees et quelles sont les limites importantes.

Important: l'application produit des indicateurs d'analyse. Elle ne produit pas de recommandation financiere garantie. Les signaux `BUY`, `WATCH` et `AVOID` sont des classifications quantitatives internes, pas des conseils d'investissement.

## 1. Objectif

L'application sert a analyser des produits de bourse lies a un sous-jacent, par exemple Apple ou Hermes:

- warrants call et put
- turbos / open-end knock-out quand ils sont exploitables par le moteur d'opportunites
- mini-futures et produits proches pour l'analyse de decorellation
- donnees de sous-jacent, devises, bougies, frais, bid/ask, volatilite et scenario de prix

Elle repond a deux grands besoins:

1. Trouver des produits potentiellement decorelles par rapport a leurs pairs.
2. Evaluer un scenario humain: par exemple "Hermes a 2000 EUR au 31/10/2026" ou "Apple a 330 USD au 31/10/2026".

## 2. Architecture rapide

Le projet est une application Rust executee via Docker.

Fichiers principaux:

- `manage.sh`: interface de lancement Docker.
- `src/main.rs`: point d'entree CLI.
- `src/api/`: recuperation des donnees externes.
- `src/models/`: structures de donnees normalisees.
- `src/indicators/`: calculs de valorisation, Black-Scholes, signaux warrants.
- `src/decision/`: decision `BUY/WATCH/AVOID`, risque, frais, scenario, stops et targets.
- `src/display/`: affichage Ratatui et exports texte/CSV.
- `src/llm/gemini.rs`: audit qualitatif Gemini dans le mode scenario.
- `data/cache/`: caches locaux, notamment bougies et LLM.
- `.env`: configuration locale.
- `.env.example`: exemple de configuration.

Flux simplifie:

```text
Boursorama / Euronext / Yahoo / APIs optionnelles
        |
        v
Normalisation produits + spot + FX + bougies
        |
        v
Nettoyage de donnees: bid/ask, devise, parite, maturite, statut
        |
        v
Indicateurs: intrinsic, spread, IV, Black-Scholes, gap, proba, EV
        |
        v
Decision: BUY / WATCH / AVOID
        |
        v
Ratatui / CSV / audit Gemini
```

## 3. Installation et build

Depuis le dossier du projet:

```bash
./manage.sh build
```

L'image Docker construite s'appelle:

```text
warrant-fetcher
```

Pour relancer un build complet apres modification du code:

```bash
./manage.sh rebuild
```

Pour lancer les tests unitaires:

```bash
./manage.sh test-unit
```

Etat actuel attendu:

```text
90 tests passed
```

## 4. Configuration `.env`

Le fichier `manage.sh` charge automatiquement `.env`.

Une variable deja exportee dans le shell garde la priorite sur `.env`.

Exemple de base:

```env
OPPORTUNITY_UNDERLYING=hermes
UNDERLYING_TICKER=RMS.PA
OPPORTUNITY_FORMAT=tui

CANDLES_RANGE=60d
CANDLES_INTERVAL=60m
MARKET_DATA_REFRESH=cache

BROKER_FEE_PROFILE=bourse_direct_1000
FEE_ORDER_NOTIONAL=1000

SCENARIO_TARGET_PRICE=1800
SCENARIO_TARGET_DATE=2026-10-31
SCENARIO_MIN_MATURITY=2027-01-01
SCENARIO_MAX_MATURITY=2027-03-31
SCENARIO_SIDE=auto
SCENARIO_MIN_BUY_RETURN_PCT=8
SCENARIO_MIN_BUY_SCORE=70
SCENARIO_MIN_BUY_TARGET_PROB_PCT=10
SCENARIO_MIN_BUY_BREAKEVEN_PROB_PCT=20
SCENARIO_SCORE_RETURN_TARGET_PCT=20
SCENARIO_MAX_UNMODELED_LINEAR_BUY_DAYS=45
SCENARIO_LINEAR_FINANCING_RATE_PCT=0
SCENARIO_MAX_BUY_KO_PROB_PCT=30
SCENARIO_MC_PATHS=512
SCENARIO_MC_MAX_STEPS=128
SCENARIO_VOL_SHOCK_POINTS=-5

LLM_ENABLE=0
GEMINI_API_KEY=gemini_key_1,gemini_key_2
GEMINI_MODEL=gemini-3-pro-preview
LLM_CACHE=1
```

## 5. Commandes principales

### Build

```bash
./manage.sh build
```

Construit l'image Docker.

### Tests unitaires

```bash
./manage.sh test-unit
```

Lance `cargo test` dans un container Rust.

### Decouverte des produits

```bash
./manage.sh discover hermes 50
```

Cherche des produits lies au sous-jacent `hermes`.

### Analyse brute

```bash
./manage.sh analyze hermes RMS.PA 50
```

Produit une table CSV de produits normalises avec:

- side call/put
- type produit
- strike
- maturite
- parite
- prix
- emetteur
- URLs
- informations Boursorama/Euronext quand disponibles

### Recherche d'opportunites relatives

```bash
./manage.sh opportunities apple AAPL 500 all
```

ou:

```bash
./manage.sh opportunities hermes RMS.PA 500 call
```

Cette commande ouvre l'interface Ratatui si le terminal est interactif.

Filtres disponibles:

- `all`: calls et puts
- `call`: uniquement calls
- `put`: uniquement puts

Export CSV:

```bash
./manage.sh opportunities-csv apple AAPL 500 all
```

### Scenario humain

Syntaxe:

```bash
./manage.sh scenario <underlying> <ticker> <target> <target_date> <min_maturity> <max_maturity> [auto|force-call|force-put] [limit]
```

Exemple Apple:

```bash
BROKER_FEE_PROFILE=bourse_direct_1000 FEE_ORDER_NOTIONAL=1000 CANDLES_RANGE=60d CANDLES_INTERVAL=60m MARKET_DATA_REFRESH=cache ./manage.sh scenario apple AAPL 330 2026-10-31 2027-01-01 2027-03-31 500
```

Target en zone:

```bash
BROKER_FEE_PROFILE=bourse_direct_1000 FEE_ORDER_NOTIONAL=1000 CANDLES_RANGE=1y CANDLES_INTERVAL=1d MARKET_DATA_REFRESH=cache ./manage.sh scenario apple AAPL 365..375 2026-10-31 2027-01-01 2028-12-31 auto 500
```

La syntaxe accepte aussi `365:375`, `365;375`, ou les variables:

```env
SCENARIO_TARGET_RANGE=365..375
SCENARIO_TARGET_MIN=365
SCENARIO_TARGET_MAX=375
```

Avec une range, `target_price` devient le milieu de zone pour les projections principales. Le moteur calcule aussi les projections au bas et au haut de la zone. Par defaut, la decision reste basee sur le milieu de zone pour conserver le comportement d'une cible unique:

- `Net@D` et `P/L@D`: milieu de zone;
- `Str@D`: pire rendement stressé de la zone;
- `Tch<=D`: probabilite first-touch d'entrer dans la zone, donc bas de zone pour une these haussiere et haut de zone pour une these baissiere;
- Notes: affichage min/moy/max de rendement net sur la zone.

Pour faire influencer la range sur la decision:

```env
SCENARIO_RANGE_DECISION_MODE=avg
SCENARIO_RANGE_DECISION_MODE=worst
```

Exemple Hermes haussier:

```bash
BROKER_FEE_PROFILE=bourse_direct_1000 FEE_ORDER_NOTIONAL=1000 CANDLES_RANGE=60d CANDLES_INTERVAL=60m MARKET_DATA_REFRESH=cache ./manage.sh scenario hermes RMS.PA 2000 2026-10-31 2026-12-01 2027-12-31 500
```

Exemple Hermes baissier:

```bash
BROKER_FEE_PROFILE=bourse_direct_1000 FEE_ORDER_NOTIONAL=1000 CANDLES_RANGE=60d CANDLES_INTERVAL=60m MARKET_DATA_REFRESH=cache ./manage.sh scenario hermes RMS.PA 1300 2026-10-31 2027-01-01 2027-03-31 500
```

Le mode par defaut (`auto`) suit la direction de la these. Il garde les produits haussiers quand le target est au-dessus du spot et les produits baissiers quand le target est sous le spot:

- target au-dessus du spot: probabilites first-touch calculees comme scenario haussier
- target sous le spot: probabilites first-touch calculees comme scenario baissier

Forcer une direction ou analyser les deux cotes:

```bash
./manage.sh scenario hermes RMS.PA 1700 2026-10-31 2027-01-01 2027-03-31 force-put 500
./manage.sh scenario hermes RMS.PA 1700 2026-10-31 2027-01-01 2027-03-31 mixed 500
```

`mixed`/`all` garde les calls et puts ensemble. `SCENARIO_ALLOW_OPPOSITE_SIDE=1` permet aussi de conserver les produits opposes en mode `auto`, mais c'est un mode d'audit, pas le comportement recommande.

### Bougies et cache

Telecharger ou rafraichir les bougies:

```bash
./manage.sh candles AAPL 6mo 1d refresh
```

Utiliser le cache local:

```bash
./manage.sh candles AAPL 6mo 1d cache
```

Pour Hermes avec bougies plus fines:

```bash
./manage.sh candles RMS.PA 60d 60m refresh
```

Puis relancer l'analyse avec:

```bash
CANDLES_RANGE=60d CANDLES_INTERVAL=60m MARKET_DATA_REFRESH=cache ./manage.sh scenario hermes RMS.PA 1300 2026-10-31 2027-01-01 2027-03-31 500
```

## 6. Interface Ratatui

### Opportunities

Touches:

- `a`: all
- `c`: calls
- `p`: puts
- `u`: sous-evalues
- `o`: sur-evalues
- `b`: afficher `BUY`
- `w`: afficher `WATCH`
- `x`: afficher `AVOID`
- `d`: toutes decisions
- `Enter`: ouvrir Boursorama
- fleches haut/bas: changer de ligne
- fleches gauche/droite: scroll notes ligne par ligne
- `Tab` / `Shift+Tab`: scroll notes par page
- `n` / `p`: scroll notes
- `q`: quitter

Colonnes principales:

- `Dec`: decision interne
- `Conf`: confiance
- `Edge`: avantage net apres spread
- `R/R`: reward/risk
- `Size`: taille indicative
- `Stop`: perte au stop
- `T1`, `T2`: objectifs
- `BE`: breakeven
- `Fee`: impact des frais
- `Spr`: spread
- `DQ`: data quality
- `Flow`: flux/volume informatif, non decisif
- `IV`, `IVd`: volatilite implicite et ecart au smile quand disponible

### Scenario

Touches:

- `b`: filtrer BUY
- `w`: filtrer WATCH
- `x`: filtrer AVOID
- `d`: toutes decisions
- `Enter`: ouvrir Boursorama
- `m`: modifier la cible dans l'application
- `l`: basculer Notes / LLM
- `r`: interroger Gemini
- fleches haut/bas: changer de produit
- fleches gauche/droite: scroll notes ligne par ligne
- `Tab` / `Shift+Tab`: scroll notes par page
- `n` / `p`: scroll notes
- `q`: quitter

Quand `m` est utilise, l'application recalcule:

- le cote coherent avec la nouvelle cible si `SCENARIO_SIDE=auto`
- les prix projetes
- les probabilites
- les decisions
- l'affichage

`SCENARIO_SIDE=auto` deduit le cote a partir de la cible. Une these haussiere garde les produits haussiers, une these baissiere garde les produits baissiers. Utilise `mixed`/`all` ou `SCENARIO_ALLOW_OPPOSITE_SIDE=1` pour analyser volontairement les deux cotes; dans ce cas les probabilites `Tch<=D` restent calculees dans la direction de la these de marche, pas dans le sens nominal du warrant.

Colonnes utiles du mode scenario:

- `Net@D`: rendement net projete a la date cible, apres frais broker et spread de sortie estime. Avec une range, c'est le milieu de zone par defaut.
- `P/L@D`: gain/perte en euros a la date cible pour `FEE_ORDER_NOTIONAL`. Avec une range, c'est le milieu de zone par defaut.
- `Str@D`: rendement net a la date cible si l'IV baisse de `SCENARIO_VOL_SHOCK_POINTS`. Avec une range, c'est le pire stress de la zone.
- `BE mv`: mouvement minimum du sous-jacent pour atteindre le point mort
- `Tch<=D`: probabilite first-touch d'atteindre la cible avant ou a la date scenario
- `KO%`: probabilite first-touch de barriere/knock-out avant la date cible, si barriere connue
- `MCev`: esperance de rendement de la simulation Monte Carlo target/stop/KO
- `Type`: famille/model du produit (`Warrant`, `Turbo`, `MiniF`, `KO-fin`, `KO-bar`)
- `EntryAsk`: prix d'entree acheteur utilise
- `ExitBid@D`: prix de sortie bid estime a la date cible, hors frais broker
- `IVout`: IV de sortie pour warrant vanilla; `linear` pour produit lineaire sans IV/theta/vega Black-Scholes
- `Strike`, `Maturite`, `Par`: caracteristiques du contrat a verifier avant toute decision

Quand l'ecran est assez large, le panneau Notes affiche aussi une droite de decision:

- axe: mouvement du sous-jacent en % depuis le spot courant
- `S`: spot actuel, point d'entree normalise a `0%`
- `X`: stop loss theorique mark-to-market, calcule avec `DECISION_MAX_LOSS_PCT_PER_TRADE`
- `B`: point mort a la date cible, niveau a partir duquel le trade devient rentable apres frais
- `T`: target/TP du scenario, avec rendement net apres frais

La droite n'est pas une prediction du chemin du prix. Elle sert a lire l'ordre des seuils: stop, spot, breakeven et target. Elle permet de voir tout de suite si le target est avant ou apres le point mort.

Le point mort `B` depend du modele du produit. Pour un warrant vanilla, il est calcule avec Black-Scholes a la date cible:

```text
net_return(BlackScholes(S_BE, K, T_target_to_maturity, r, q, vol) / parite * fx) = 0
```

Pour un turbo, mini-future ou knock-out a financement, il est calcule par projection lineaire:

```text
intrinsic = max(direction * (S_BE - niveau_financement), 0)
prix = intrinsic / parite * fx
```

Ces produits ne recoivent pas de theta/vega Black-Scholes artificiels dans le scenario. Leur risque principal vient plutot du niveau de financement, de la barriere, du spread, du FX et du financement implicite de l'emetteur. Si le sous-jacent atteint le niveau plus tot ou plus tard que la date cible, le point mort reel peut etre different.

Le niveau de financement/barriere futur est projete si `SCENARIO_LINEAR_FINANCING_RATE_PCT` est renseigne. Pour un open-end call, la reference augmente avec le cout de financement; pour un open-end put, elle diminue. La formule interne est:

```text
call reference_future = reference_now * exp(financing_rate * T)
put  reference_future = reference_now * exp(-financing_rate * T)
```

Si ce taux reste a `0`, le moteur utilise la reference actuelle. Pour une these longue sur plusieurs mois ou annees, il faut donc lire le resultat comme une approximation du payoff lineaire, pas comme une promesse de prix futur exact.

Par defaut, un produit lineaire open-end dont le target est a plus de `SCENARIO_MAX_UNMODELED_LINEAR_BUY_DAYS` jours ne peut pas etre classe `BUY` si `SCENARIO_LINEAR_FINANCING_RATE_PCT=0`. Il reste affiche, mais il passe au minimum en `WATCH` avec la raison `linear_financing_unmodeled_long_horizon`. Ce garde-fou evite de transformer une projection lineaire propre a court terme en faux signal fort sur plusieurs mois, car le niveau de financement futur n'est pas encore connu.

### Barriere / KO

Si une barriere est connue, le moteur calcule une probabilite first-touch avant la date cible:

```text
KO% = P(S_t touche la barriere avant target_date)
```

Pour un produit haussier avec barriere basse, on calcule une first-touch baissiere. Pour un produit baissier avec barriere haute, on calcule une first-touch haussiere. Si `KO% > SCENARIO_MAX_BUY_KO_PROB_PCT`, le moteur ajoute `barrier_touch_probability_too_high` et bloque `BUY`.

### Monte Carlo

Le moteur simule des trajectoires GBM:

```text
S_next = S * exp((mu - 0.5 * sigma^2) * dt + sigma * sqrt(dt) * Z)
```

Puis il observe dans chaque trajectoire:

- target touche avant stop/KO
- stop touche avant target
- KO touche avant target
- P/L final si rien n'est touche avant la date cible

Les champs affiches sont:

- `MCev`: moyenne des P/L simules avec target, stop mark-to-market et KO
- `p05`, `p50`, `p95` dans les notes: percentiles pessimiste, median et favorable
- `target_first`, `stop_first`, `KO` dans les notes

Si `MCev < 0`, le moteur ajoute `monte_carlo_ev_negative`. Si le stop mark-to-market/KO arrive au moins aussi souvent que le target, il ajoute `target_before_stop_not_favored`.

## 7. Concepts financiers utilises

### Bid, ask et spread

Le `bid` est le prix de rachat.

Le `ask` est le prix d'achat.

Le spread mesure le cout implicite d'entree/sortie:

```text
mid = (bid + ask) / 2
spread_pct = (ask - bid) / mid * 100
```

Un spread de `20%` est souvent bloquant: le trade commence avec un gros handicap.

### Parite

La parite indique combien de warrants representent une unite du sous-jacent.

Exemple:

```text
Parite 10 = 10 warrants pour 1 action
Parite 100 = 100 warrants pour 1 action
```

La parite est essentielle dans tous les calculs:

```text
valeur_par_warrant = valeur_sous_jacent / parite
```

### Devises

Le moteur convertit les prix pour eviter de melanger EUR et USD.

Exemple Apple:

```text
Sous-jacent AAPL en USD
Produit cote en EUR
FX USD->EUR utilise pour projeter le prix du produit
```

### Valeur intrinseque

Pour un call:

```text
intrinsic = max(S - K, 0)
```

Pour un put:

```text
intrinsic = max(K - S, 0)
```

Par produit:

```text
intrinsic_per_product = intrinsic / parite * fx
```

### Warrants vanilla et Black-Scholes

Pour les warrants classiques, le mode scenario utilise Black-Scholes.

Projection:

```text
prix_projete = BlackScholes(side, S_target, K, T_target_to_maturity, r, q, vol) / parite * fx
```

Ou:

- `S_target`: prix vise du sous-jacent a la date cible
- `K`: strike
- `T_target_to_maturity`: temps restant entre la date cible et la maturite
- `r`: taux sans risque
- `q`: rendement dividende
- `vol`: volatilite implicite si disponible, sinon fallback

Ce calcul integre deja:

- le compte a rebours: `T_target_to_maturity` diminue quand la date cible avance, donc la valeur temps restante baisse
- le delta: Black-Scholes ne suppose pas un levier fixe; le prix reagit selon la position du warrant face au strike

### Turbos, mini-futures et open-end

Les produits lineaires compatibles sont maintenant gardes en mode `scenario`:

- `turbo / financing_level`
- `mini_future / financing_level`
- `open_end_knock_out / financing_level`
- `open_end_knock_out / barrier_only`

Le moteur les projette avec leur valeur intrinseque future:

```text
call: max(S_target - reference, 0) / parite * fx
put:  max(reference - S_target, 0) / parite * fx
```

Si une barriere est fournie et que le target la traverse du mauvais cote, le prix projete tombe a `0`. Pour les produits `open-end`, la maturite affichee reste `open-end`; une date interne synthétique sert seulement aux calculs auxiliaires, et le theta est force a `0`.
- le vega: le prix depend de `vol`

Le moteur ajoute aussi un stress de volatilite:

```text
vol_stress = max(vol + SCENARIO_VOL_SHOCK_POINTS / 100, 0.0001)
prix_stress = BlackScholes(side, S_target, K, T_target_to_maturity, r, q, vol_stress) / parite * fx
```

Avec la valeur par defaut `SCENARIO_VOL_SHOCK_POINTS=-5`, le panneau Notes affiche le rendement au target si l'IV baisse de 5 points. Si ce rendement devient negatif, la raison `volatility_crush_erases_return` bloque le `BUY`.

### Probabilites

Le moteur estime:

- `P(target)`: probabilite d'atteindre le target a la date cible
- `P(BE)`: probabilite d'atteindre le breakeven a la date cible

Formule lognormale:

```text
z = (ln(level / spot) - (r - q - 0.5 * vol^2) * T) / (vol * sqrt(T))
```

Pour un call:

```text
P(level atteint) = 1 - Phi(z)
```

Pour un put:

```text
P(level atteint) = Phi(z)
```

### Fair-value risk-neutral

La valeur affichee historiquement comme `EV` est une mesure de fair-value risk-neutral, pas une vraie esperance monde reel:

```text
fair_now = BlackScholes(spot actuel, strike, maturite)
expected_exit = fair_now * exp(r * T_to_target)
Fair-value Q = rendement net de expected_exit vs prix d'entree apres frais
```

Si cette valeur est negative, le moteur ajoute `expected_value_negative`. Le produit peut rester `WATCH` si le scenario cible est interessant, mais il ne doit pas sortir en `BUY`: cela signifie que le prix d'entree est cher selon le modele risk-neutral, meme si le payoff conditionnel au target parait attractif.

Elle ne represente pas une conviction humaine. Elle sert a comparer le prix du produit avec une valorisation theorique.

### Kelly actuel

Le Kelly affiche est un Kelly binaire au target:

```text
Kelly = max(0, (b * p - q) / b)
b = gain net si le target est atteint
p = P(target)
q = 1 - p
perte supposee = -100%
```

Ce Kelly est volontairement conservateur.

Il reste souvent a `0.00%` quand:

```text
P(target) < 1 / (1 + gain_net)
```

Exemple:

```text
Gain net +110%
P requise = 1 / 2.10 = 47.6%
Si P(target) = 14%, Kelly = 0
```

Limite actuelle: ce Kelly ne modelise pas encore un vrai plan de sortie avec stop, prise partielle, trailing stop ou sortie avant target.

## 8. Data quality et Flow

### Data quality

`DQ` mesure la qualite exploitable du prix:

- bid/ask disponible
- ask non nul
- prix coherent
- devise connue
- parite connue
- statut de cotation
- absence de donnees obsoletes ou aberrantes

Une data quality faible peut forcer `AVOID`.

### Flow

`Flow` est informatif uniquement.

Sur ces produits, l'investisseur traite souvent contre le market maker ou l'emetteur, pas contre un carnet entre particuliers. Le volume faible n'est donc plus une raison automatique d'eviter.

Les vrais criteres d'execution sont:

- bid/ask executable
- spread
- tailles affichees si disponibles
- statut de cotation
- fraicheur des donnees

## 9. Decisions BUY, WATCH, AVOID

### BUY

Le moteur peut classer `BUY` si:

- rendement ou edge suffisant
- spread acceptable
- donnees propres
- frais integres
- reward/risk suffisant
- pas de raison bloquante

En mode `scenario`, les seuils sont configurables:

```env
SCENARIO_MIN_BUY_RETURN_PCT=8
SCENARIO_MIN_BUY_SCORE=70
SCENARIO_MIN_BUY_TARGET_PROB_PCT=10
SCENARIO_MIN_BUY_BREAKEVEN_PROB_PCT=20
SCENARIO_SCORE_RETURN_TARGET_PCT=20
SCENARIO_MAX_UNMODELED_LINEAR_BUY_DAYS=45
SCENARIO_LINEAR_FINANCING_RATE_PCT=0
SCENARIO_MAX_BUY_KO_PROB_PCT=30
SCENARIO_MC_PATHS=512
SCENARIO_MC_MAX_STEPS=128
SCENARIO_VOL_SHOCK_POINTS=-5
```

Cela evite de forcer uniquement des scenarios tres lointains. Un mouvement du sous-jacent de 3-5% peut donc produire un `BUY` si le produit donne un rendement net suffisant, un spread faible, une data quality propre et une probabilite correcte.

`SCENARIO_VOL_SHOCK_POINTS=-5` applique un stress de volatilite implicite de `-5 points` au target. Il sert a detecter le cas classique ou le sous-jacent va dans le bon sens mais ou la baisse d'IV efface la plus-value du warrant.

### WATCH

`WATCH` signifie:

- idee interessante
- mais conditions imparfaites
- attendre meilleur spread, meilleure donnee, meilleure entree ou confirmation

### AVOID

`AVOID` peut venir de:

- spread trop large
- prix d'entree non executable
- data quality trop faible
- rendement negatif
- breakeven au-dela du target
- maturite avant/apres la fenetre souhaitee
- produit non compatible avec le moteur scenario

## 10. Mode Opportunities

Le mode `opportunities` cherche des decorellations relatives.

Pour les produits a financement / knock-out:

```text
metric = price_to_intrinsic = prix / valeur_intrinseque_par_produit
gap = (median_peer_metric - metric) / median_peer_metric * 100
edge = gap - spread
```

Pour les warrants vanilla:

Le moteur utilise une logique de prime / breakeven plutot qu'un simple ratio prix/intrinseque, car un warrant a de la valeur temps.

Call:

```text
premium_call_pct = ((K + prix_ref_usd * parite - S) / S) * 100
```

Put:

```text
premium_put_pct = ((S + prix_ref * parite - K) / S) * 100
```

Le prix est converti dans la devise du sous-jacent avant le calcul.

## 11. Mode Scenario

Le mode `scenario` ne suppose pas une sortie a maturite.

Il suppose:

```text
entree maintenant
sous-jacent = target a target_date
valorisation du warrant a target_date
temps restant jusqu'a maturite conserve
```

Exemple:

```text
Apple vaut 297 aujourd'hui
Target 330 au 2026-10-31
Warrant expire le 2027-03-19
Le moteur valorise le warrant au 2026-10-31 avec une maturite restante jusqu'au 2027-03-19
```

Ce mode sert a repondre:

```text
Si ma these de prix se realise a telle date, quel produit donne le meilleur profil apres frais et risques ?
```

## 12. Frais broker

Les frais sont integres via:

- `BROKER_FEE_PROFILE`
- `FEE_ORDER_NOTIONAL`
- `FEE_BUY_FIXED`
- `FEE_BUY_PCT`
- `FEE_SELL_FIXED`
- `FEE_SELL_PCT`
- `FEE_DEPOSIT_FIXED`
- `FEE_DEPOSIT_PCT`

Profiles disponibles dans `.env.example`:

- `custom`
- `trade_republic`
- `bourse_direct_500`
- `bourse_direct_1000`
- `bourse_direct_2000`
- `bourse_direct_pct`
- `bourse_direct_morgan_stanley`
- `degiro_fr_actions`
- `degiro_otc_sg_bnp`
- `fortuneo_starter`
- `fortuneo_starter_first_500`

Exemple:

```bash
BROKER_FEE_PROFILE=bourse_direct_1000 FEE_ORDER_NOTIONAL=1000 ./manage.sh scenario apple AAPL 330 2026-10-31 2027-01-01 2027-03-31 500
```

## 13. LLM Gemini

Le mode scenario peut interroger Gemini avec la touche `r`.

Variables:

```env
LLM_ENABLE=1
GEMINI_API_KEY=cle_1,cle_2
GEMINI_MODEL=gemini-3-pro-preview
LLM_CACHE=1
```

Les cles separent par virgule sont testees en rotation si une cle echoue ou est rate-limitee.

Gemini recoit:

- le scenario
- le sous-jacent
- le produit selectionne
- les prix
- les frais
- les probabilites first-touch et terminales
- les greeks actuels et projetes
- les stress IV et FX
- les dividendes discrets connus
- les raisons BUY/WATCH/AVOID
- le flow en champ informatif uniquement, non decisif

Regles importantes:

- Gemini ne doit pas utiliser `Flow` comme raison de decision.
- Gemini doit expliquer les drivers, red flags, checks d'execution et conditions d'invalidation.
- L'avis LLM est qualitatif.
- Le moteur quantitatif reste la source des calculs.
- Les reponses Gemini sont cachees si `LLM_CACHE=1`.
- Le cache LLM est indexe par SHA-256 sur le modele et les prompts; changer de modele ou de regles cree donc une nouvelle entree de cache.
- Les blocages Gemini de type `promptFeedback.blockReason` sont affiches explicitement dans l'onglet LLM.

Si une ancienne reponse reste affichee dans une session TUI, relancer l'application ou lancer une analyse avec:

```bash
LLM_CACHE=0 ./manage.sh scenario ...
```

## 14. APIs et fournisseurs

Sources actuellement utilisees ou prevues:

- Yahoo chart: bougies et spot selon les cas.
- Boursorama: pages produits, bid/ask, detail produit.
- Euronext: listes et details de produits.
- ORATS: donnees options si cle configuree.
- Polygon: donnees de marche si cle configuree.
- Gemini: audit LLM qualitatif.

Variables API:

```env
ORATS_API_KEY=key1,key2
POLYGON_API_KEY=key1,key2
GEMINI_API_KEY=key1,key2
```

## 15. Cache

Bougies:

```text
data/cache/candles
```

LLM:

```text
data/cache/llm
```

Modes bougies:

- `cache`: lit le cache si disponible, sinon telecharge.
- `refresh`: force le telechargement et remplace le cache.

## 16. Debug

Pour comprendre les lenteurs du mode opportunities:

```bash
OPPORTUNITY_DEBUG=1 OPPORTUNITY_DEBUG_EVERY=25 ./manage.sh opportunities apple AAPL 500 all
```

En debug, l'affichage peut basculer en table pour rendre les logs lisibles.

## 17. Tests couverts

La suite de tests couvre notamment:

- parsing Boursorama
- bid/ask avec ask a zero
- rejet des prix non executables
- conversion EUR/USD
- parite
- valeur intrinseque calls/puts
- produits knock-out / mini-future
- Black-Scholes
- volatilite implicite
- greeks
- scenario target
- frais broker
- cache bougies
- rotation de cles API
- nettoyage des reponses Gemini
- absence de decision basee sur `Flow`

Commande:

```bash
./manage.sh test-unit
```

## 18. Limites actuelles

Limites importantes:

- Le moteur scenario traite les warrants vanilla compatibles Black-Scholes et les produits lineaires a financement/barriere.
- Les bonus, discounts, express, certificats a payoff conditionnel ou produits exotiques ne sont pas encore modelises en scenario.
- Le Kelly actuel est binaire et tres conservateur.
- Le moteur ne modelise pas encore une strategie complete avec stop dynamique, prise partielle, sortie temporelle et trailing stop.
- Les stops prives, liquidations reelles et gros ordres caches ne sont pas observables directement sur actions.
- Les donnees Boursorama peuvent etre decalees ou incompletes.
- Un ask a `0.000` ou un bid absent invalide l'executabilite.
- Un spread large peut rendre un produit mathematiquement interessant mais mauvais en pratique.

## 19. Interpreting les cas frequents

### Beaucoup de AVOID

Ca signifie souvent:

- spread trop large
- data quality faible
- target trop ambitieux
- peu de produits disponibles
- maturite hors fenetre
- prix non executable

### Kelly a 0

Normal si:

```text
P(target) est inferieur a la probabilite minimale requise par le gain net.
```

Le champ `p requis` dans les notes explique ce seuil.

### Un produit avec +100% net reste AVOID

Possible si:

- spread tres large
- probabilite target faible
- data quality trop basse
- Kelly nul
- prix d'achat pas propre

Un gros rendement potentiel ne suffit pas.

### Flow faible

Ce n'est plus une raison de rejet. C'est seulement informatif.

Pour les produits de bourse, l'execution depend surtout du market maker:

- bid
- ask
- spread
- taille affichee
- statut de cotation

## 20. Prochaines evolutions logiques

Les evolutions les plus utiles:

1. Moteur de strategie distinct du scenario:
   - take profit 1
   - take profit 2
   - stop loss
   - trailing stop
   - sortie avant maturite
   - revalorisation quotidienne

2. Kelly strategie:
   - probabilite de stop
   - probabilite de TP1
   - probabilite de TP2
   - perte controlee au lieu de perte binaire -100%

3. Module positioning court terme:
   - volume profile
   - niveaux high/low
   - VWAP
   - open interest options
   - gamma exposure
   - order book si provider disponible

4. Amelioration du support produits:
   - turbos en mode scenario
   - certificats bonus/cappes
   - produits a barriere complexes

5. Historique local des signaux:
   - comparer un signal dans le temps
   - detecter amelioration du spread
   - suivre evolution de IV et score
