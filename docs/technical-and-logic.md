# Documentation technique et logique

Ce document reprend l'etat actuel du projet `warrant-fetcher`: ce qui a ete construit, pourquoi, comment les donnees circulent, et quelle logique financiere est appliquee pour analyser les warrants, turbos, mini-futures et produits structures lies a un sous-jacent comme Apple.

> Ce projet produit des signaux d'analyse. Il ne produit pas des recommandations d'achat/vente et ne prouve pas un arbitrage executable. Les frais, le spread bid/ask, le change, la liquidite, la fiscalite, le statut de cotation et les conditions de l'emetteur doivent etre controles avant toute decision.

## Objectif

Le but est de construire une base propre et modulaire pour:

- recuperer le prix actuel du sous-jacent, par exemple `RMS.PA`;
- recuperer la liste des produits structures Euronext lies au sous-jacent;
- enrichir chaque produit avec un maximum de caracteristiques utiles;
- separer proprement calls, puts, maturites, strikes, parites, emetteurs et types de produits;
- calculer des indicateurs exploitables;
- afficher les signaux dans une interface claire, notamment avec Ratatui;
- eviter les faux signaux dus aux produits expires, suspendus, sans prix ou mal compares.

## Sources de donnees

### Yahoo Finance

Utilise pour recuperer le prix du sous-jacent, via le module historique du projet:

- ticker spot: `RMS.PA`;
- prix;
- devise;
- variation en pourcentage.

Ce prix sert de reference pour calculer:

- la moneyness;
- la valeur intrinseque;
- la distance au strike ou a la barriere;
- les comparaisons relatives entre produits.

### Euronext

Euronext est la source principale pour les produits structures.

Deux niveaux sont utilises:

1. Annuaire des produits:
   - symbole;
   - ISIN;
   - MIC;
   - nom produit;
   - sous-jacent;
   - type produit;
   - strike;
   - maturite;
   - dernier prix affiche;
   - date/heure du dernier prix.

2. `instrumentDetail`:
   - emetteur;
   - devise;
   - date d'emission;
   - prix d'emission;
   - strike 1;
   - strike 2;
   - devise des strikes;
   - parite;
   - nombre de warrants par sous-jacent;
   - levier;
   - barriere / seuil bas;
   - prix d'ouverture;
   - cloture;
   - cloture precedente;
   - haut / bas;
   - volume;
   - nombre de trades;
   - statut de cotation;
   - statut de trading;
   - horaires;
   - DIC/KID quand disponible;
   - ISIN du sous-jacent quand disponible.

### Boursorama

Boursorama expose parfois des donnees plus pratiques cote affichage:

- bid;
- ask;
- taille bid;
- taille ask;
- mid price;
- dernier prix;
- haut / bas;
- cloture precedente;
- volume;
- date de trade.

L'enrichissement Boursorama existe comme option, car scraper chaque page HTML est beaucoup plus lourd que l'API Euronext.

Activation:

```bash
BOURSORAMA_ENRICH=1 ./manage.sh analyze hermes RMS.PA 50
```

ou via Docker:

```bash
docker run --rm \
  -e ANALYZE_UNDERLYING=hermes \
  -e UNDERLYING_TICKER=RMS.PA \
  -e ANALYZE_LIMIT=50 \
  -e BOURSORAMA_ENRICH=1 \
  warrant-fetcher
```

## Architecture du code

### Modules principaux

- `src/main.rs`
  - point d'entree;
  - commandes `discover`, `analyze`, `opportunities`;
  - export CSV;
  - selection TUI/table/csv.

- `src/analysis.rs`
  - construit la base marche complete;
  - recupere le spot;
  - decouvre les produits Euronext;
  - enrichit chaque produit avec `instrumentDetail`;
  - enrichit optionnellement via Boursorama.

- `src/api/discover.rs`
  - appelle l'annuaire Euronext;
  - parse les lignes HTML de l'annuaire;
  - recupere `instrumentDetail`.

- `src/api/boursorama.rs`
  - construit l'URL courte Boursorama `1rP{symbol}`;
  - lit le JSON embarque `data-ist-init`;
  - extrait bid/ask, tailles et OHLC.

- `src/models/structured.rs`
  - modele normalise du produit;
  - direction call/put;
  - maturite;
  - prix cote;
  - details enrichis;
  - donnees Boursorama optionnelles.

- `src/models/euronext.rs`
  - structures de deserialisation Euronext;
  - champs bruts de l'API.

- `src/indicators/warrant.rs`
  - logique actuelle de classement relatif;
  - calcul de valeur intrinseque;
  - prise en compte de la parite;
  - separation sous-evalue / sur-evalue.

- `src/pricing.rs`
  - classification des familles de produits;
  - selection du modele de pricing;
  - choix de la reference de payoff: strike, barriere ou niveau de financement.

- `src/display/opportunities.rs`
  - interface Ratatui pour les candidats;
  - navigation;
  - filtres call/put;
  - vue sous-evaluee / sur-evaluee.

- `src/display/headless.rs`
  - execution sans TUI pour Docker/logs.

## Commandes

Construire l'image:

```bash
./manage.sh build
```

Tester un ticker simple:

```bash
./manage.sh test RMS.PA
```

Lister les produits Euronext:

```bash
./manage.sh discover hermes 50
```

Construire la base enrichie:

```bash
./manage.sh analyze hermes RMS.PA 50
```

Chercher des ecarts relatifs:

```bash
./manage.sh opportunities hermes RMS.PA 500
```

Filtrer seulement les calls:

```bash
./manage.sh opportunities hermes RMS.PA 500 call
```

Filtrer seulement les puts:

```bash
./manage.sh opportunities hermes RMS.PA 500 put
```

Exporter les opportunites en CSV:

```bash
./manage.sh opportunities-csv hermes RMS.PA 500 all
```

Recuperer les bougies avec cache local:

```bash
./manage.sh candles RMS.PA 6mo 1d cache
./manage.sh candles RMS.PA 6mo 1d refresh
```

Lancer les tests unitaires:

```bash
./manage.sh test-unit
```

## Moteur de decision actuel

La commande `opportunities` ne se limite plus au gap relatif. Elle construit maintenant un `DecisionSignal`:

```text
OpportunitySignal
  -> stop sous-jacent
  -> projection prix produit au stop
  -> targets T1/T2
  -> reward/risk
  -> horizon / theta
  -> sizing position
  -> decision BUY / WATCH / AVOID
```

Les garde-fous principaux sont:

- pas d'entree si `bid/ask` non executable;
- pas d'entree si `ask == 0`, `bid == 0` ou `ask < bid`;
- pas d'entree si le prix vient de `last_unverified`;
- pas d'entree si la parite manque;
- pas d'entree si le spread depasse `DECISION_MAX_SPREAD_PCT`;
- pas d'entree si la maturite est inferieure a `DECISION_MIN_MATURITY_DAYS`;
- pas d'entree si la barriere est trop proche;
- pas d'entree si le stop ou la target ne peuvent pas etre projetes.

### Colonnes de decision

| Colonne | Sens |
| --- | --- |
| `Dec` | `BUY`, `WATCH` ou `AVOID`. |
| `Conf` | Score global pondere par data, liquidite, spread, pairs, edge, R/R, barriere et temps. |
| `Edge` | Gap relatif net du spread. |
| `R/R` | Reward/risk sur la target 2. |
| `Size` / `size%` | Pourcentage de capital suggere par le module de sizing. |
| `Stop` / `stop%` | Perte estimee si le stop sous-jacent est touche. |
| `T1`, `T2` | Gains estimes aux objectifs 1 et 2. |
| `BE` | Mouvement minimal du sous-jacent pour absorber le spread. |
| `DQ` | Qualite de donnee apres nettoyage bid/ask/timestamp/statut. |
| `Liq` | Score de liquidite base sur spread, tailles et volume quand disponible. |
| `IV`, `IVd` | IV et ecart au smile si disponibles, surtout pour warrants. |

### Sizing position

Le sizing part du risque maximal accepte sur le compte.

Variables:

```text
DECISION_ACCOUNT_RISK_PCT=1
DECISION_MAX_POSITION_NOTIONAL_PCT=10
```

Formule:

```text
taille_position_% = min(
  DECISION_ACCOUNT_RISK_PCT / stop_loss_pct * 100,
  DECISION_MAX_POSITION_NOTIONAL_PCT
)
```

Exemple:

```text
stop_loss_pct = 20%
risque compte = 1%
taille = 1 / 20 * 100 = 5% du capital
```

Le CSV expose aussi `products_per_1000_account`, qui donne le nombre de produits approximatif pour 1000 unites de capital.

### Horizon et theta

Pour les turbos et mini-futures, le moteur ne calcule pas de theta Black-Scholes, car leur cout principal vient plutot du financement, du spread et de la barriere.

Pour les warrants avec IV disponible:

```text
theta_daily_pct = max(0, -theta_black_scholes_journalier / prix_entree) * 100
theta_to_horizon_pct = theta_daily_pct * holding_days
```

Regles:

- si `theta_to_horizon_pct > DECISION_MAX_THETA_TO_HORIZON_PCT`, le signal est degrade en `WATCH`;
- si `theta_to_horizon_pct > 2 * DECISION_MAX_THETA_TO_HORIZON_PCT`, le signal passe en `AVOID`;
- si la maturite restante est trop courte, le signal passe en `AVOID`.

### Bougies et cache

Les bougies du sous-jacent sont recuperees via Yahoo puis sauvegardees dans:

```text
data/cache/candles
```

Par defaut, le moteur utilise le cache quand il existe:

```text
CANDLES_MODE=cache
MARKET_DATA_REFRESH=0
```

Pour forcer une mise a jour:

```bash
MARKET_DATA_REFRESH=refresh ./manage.sh opportunities hermes RMS.PA 500 all
```

Ces bougies alimentent:

- `ATR 14`;
- volatilite realisee 20 jours;
- support proche;
- resistance proche;
- stops et targets sur plusieurs semaines.

## Modele de donnees logique

Chaque produit est normalise en `StructuredProduct`.

Champs principaux:

- `symbol`: symbole court Euronext, par exemple `IQ33S`;
- `boursorama_symbol`: symbole Boursorama, par exemple `1rPIQ33S`;
- `isin`: identifiant produit;
- `mic`: marche de cotation;
- `underlying`: sous-jacent;
- `product_type`: type Euronext;
- `direction`: `call`, `put` ou `unknown`;
- `strike`: strike principal;
- `maturity`: date ou `open-end`;
- `last_price`: dernier prix connu;
- `bid_ask`: champ brut annuaire;
- `detail`: fiche detaillee Euronext;
- `boursorama_quote`: fiche quote optionnelle Boursorama.

`ProductDetail` porte la fiche enrichie:

- emetteur;
- devise;
- strike 1;
- strike 2;
- parite brute;
- `warrants_per_underlying`;
- levier;
- seuil/barriere;
- prix d'ouverture;
- cloture;
- cloture precedente;
- haut/bas;
- volume;
- statut de cotation;
- horaires;
- DIC/KID.

## Parite et nombre de titres necessaires

La parite est essentielle. Sans elle, on compare des choses qui n'ont pas la meme unite.

Exemple:

- si `parityUnderWar = 50`, il faut environ 50 warrants pour representer une unite du sous-jacent;
- si `parityFirstWarrantUnderlying = 0.02`, alors `1 / 0.02 = 50`;
- si un produit cote `1.27 EUR`, cela ne represente pas directement `spot - strike`, mais une fraction de cette exposition.

Le projet normalise cela dans:

```text
warrants_per_underlying
```

Puis:

```text
intrinsic_per_product = raw_intrinsic / warrants_per_underlying
```

Si le strike ou le niveau de financement est en USD et que le produit cote en EUR, cette valeur est ensuite convertie dans la devise de cotation du produit avant de calculer le ratio. Pour la paire EUR/USD, le projet interroge explicitement l'API Yahoo avec le ticker `EURUSD=X`; pour convertir USD vers EUR, il utilise l'inverse de ce taux.

Sans cette correction, les anciens signaux pouvaient afficher des ecarts absurdes de type `98%`.

## Logique call / put

La logique call et put est directionnelle.

Valeur intrinseque brute:

```text
Call = max(spot - strike, 0)
Put  = max(strike - spot, 0)
```

Donc:

- un call gagne quand le sous-jacent monte;
- un put gagne quand le sous-jacent baisse;
- un call est plus dans la monnaie quand son strike est sous le spot;
- un put est plus dans la monnaie quand son strike est au-dessus du spot.

Les calls et les puts ne doivent pas etre compares ensemble. Le moteur groupe donc les produits par:

- direction;
- type de produit;
- devise;
- maturite;
- zone de distance au spot.

## Moneyness

Le projet classe un produit en:

- `itm`: in the money;
- `atm`: at the money;
- `otm`: out of the money;
- `unknown`.

Pour le scoring relatif actuel, seuls les produits `itm` sont utilises. Cela evite de comparer un produit avec valeur intrinseque quasi nulle, ou essentiellement compose de valeur temps, avec un produit bien dans la monnaie.

## Types de produits et modeles

Le projet ne suppose pas qu'un warrant classique, un turbo, un mini-future ou un certificat ont exactement la meme formule. Chaque produit est d'abord classe en famille, puis un modele de calcul est choisi.

Base commune:

```text
direction
spot
strike
parite
prix
devise
maturite
statut
```

Mais la logique doit ensuite etre specialisee:

- `warrant / warrant_intrinsic`: payoff call/put classique sur le strike;
- `mini_future / financing_level`: payoff sur le niveau de financement, avec barriere separee;
- `turbo / financing_level`: payoff sur le niveau de financement quand il est disponible;
- `open_end_knock_out / financing_level`: utilise le financement si Euronext le fournit;
- `open_end_knock_out / barrier_only`: fallback sur la barriere/strike si aucun financement n'est disponible;
- `certificate / unsupported`: ignore dans le scoring relatif tant qu'un payoff dedie n'est pas implemente;
- `other / unsupported`: ignore dans le scoring relatif.

L'etape actuelle est donc une base de scoring relatif, pas encore un pricer complet.

## Scoring relatif actuel

Le scoring actuel cherche les produits decorelles par rapport a leurs pairs.

Pour chaque produit comparable:

```text
pricing_spec = famille + modele + reference de payoff
prix_utilise = ask executable si disponible, sinon dernier prix non verifie
raw_intrinsic = valeur intrinseque call/put
intrinsic_per_product = raw_intrinsic / warrants_per_underlying
intrinsic_per_product = conversion FX vers la devise du produit
price_to_intrinsic = prix_utilise / intrinsic_per_product
```

Pour garder l'interface rapide, `manage.sh opportunities` laisse `BOURSORAMA_ENRICH=0` par defaut. Sans enrichissement, un produit peut donc etre affiche avec `price_source = last_unverified`: il faut alors le considerer comme un signal de recherche, pas comme un prix d'achat executable. Si on lance `BOURSORAMA_ENRICH=1 ./manage.sh opportunities ...`, Boursorama est consulte et un carnet borgne avec `ask = 0` est exclu des candidats achetables. Ce mode est plus fiable mais beaucoup plus lent sur 500 produits.

Le parsing du carnet distingue maintenant trois cas:

- `3,140 / 3,150 EUR`: ask positif, le prix utilise est `3.150 EUR`;
- `3,620 / 0,000 EUR`: ask explicitement nul, le produit est exclu et on ne retombe pas sur le dernier prix;
- `/`: carnet inconnu cote Euronext, fallback possible vers `last_unverified`.

Quand un prix cote est en `EUR` et le strike en `USD`, les devises restent separees: le prix utilise porte sa devise de cotation, puis le module FX convertit vers la devise de reference du payoff.

La metrique de comparaison depend ensuite du modele de produit.

Pour les turbos, mini-futures et knock-out a financement, on conserve:

```text
relative_metric = price_to_intrinsic
```

Pour les warrants vanille, `price_to_intrinsic` n'est pas une bonne metrique de ranking: un warrant contient une valeur temps, et peut donc valoir beaucoup plus que sa seule valeur intrinseque jusqu'a l'echeance. On utilise donc la prime de break-even, avec le prix du warrant converti dans la devise de reference du sous-jacent:

```text
warrant_price_ref = prix_warrant converti vers la devise strike/sous-jacent

premium_call_pct =
  (strike + warrant_price_ref * warrants_per_underlying - spot)
  / spot
  * 100

premium_put_pct =
  (spot + warrant_price_ref * warrants_per_underlying - strike)
  / spot
  * 100

relative_metric = premium_pct
```

Cette prime indique le mouvement necessaire du sous-jacent pour atteindre le point mort a maturite. Elle ne remplace pas un modele theorique complet, mais evite de prendre la valeur temps normale d'un warrant pour une anomalie.

Ensuite, dans chaque groupe de pairs:

```text
median = median(relative_metric des pairs)
gap_pct = (median - relative_metric) / median * 100
```

Interpretation:

- `gap_pct > 0`: le produit est moins cher que la mediane de ses pairs, donc candidat sous-evalue;
- `gap_pct < 0`: le produit est plus cher que la mediane, donc candidat sur-evalue;
- `score = abs(gap_pct)`.

Exemple:

```text
relative_metric = 1.1158
median = 1.2310
gap_pct = (1.2310 - 1.1158) / 1.2310 * 100 = +9.36%
```

Dans la TUI, la zone `Notes` affiche maintenant:

- la formule de la metrique (`premium_pct` ou `price_to_intrinsic`);
- la formule du `gap`;
- apres `Entree`, l'URL Boursorama et le recapitulatif du calcul utilise.

Le moteur compare seulement des produits dans un bucket compatible:

```text
famille | modele | direction | moneyness | devise | maturite | distance a la reference
```

La largeur de la bande de distance depend du modele:

- warrants vanille: bande de `5%`, car un call tres ITM et un call ATM n'ont pas la meme structure de valeur temps;
- produits a financement/barriere: bande de `20%`, car la metrique reste proche du rapport prix / intrinseque finance.

Les produits deja arrives a maturite, y compris le jour de maturite, sont exclus du scoring relatif. Exemple: un warrant maturite `2026-05-15` ne doit plus participer a une analyse lancee le `2026-05-17`.

Le moteur affiche:

- top 10 sous-evalues;
- top 10 sur-evalues.

## Filtres d'exclusion

Les exclusions actuelles evitent les produits manifestement inutilisables.

Exclus:

- produit sans fiche detaillee;
- produit non liste;
- produit arrive a maturite ou le jour de sa maturite;
- `quotation_state = HAL`;
- `trading_status = HAL` ou `SUS`;
- prix nul ou absent;
- direction inconnue;
- produit non `itm` pour le scoring relatif.

Important: `CLO` n'est plus exclu automatiquement, car le marche peut simplement etre ferme. Si on exclut `CLO`, un run le week-end peut faire disparaitre toute la base.

## Affichage Ratatui

La vue `opportunities` utilise Ratatui si le terminal est interactif.

Touches:

- `a`: tous les produits;
- `c`: calls;
- `p`: puts;
- `u`: sous-evalues;
- `o`: sur-evalues;
- `Entree`: ouvrir la page Boursorama du produit selectionne;
- fleches ou `j/k`: navigation;
- `q` ou `Esc`: quitter.

Colonnes principales:

- `Gap`: ecart a la mediane du groupe;
- `Symbol`;
- `Side`;
- `Type`;
- `Mat.`;
- `Price`;
- `price_source` dans le CSV et le detail TUI: source du prix utilise pour la metrique;
- `Ref.`: reference de payoff utilisee pour le calcul;
- `Parity`;
- `Intr/W`: valeur intrinseque par warrant, convertie dans la devise du produit;
- `Metric`: metrique de comparaison (`premium_pct` pour warrant vanille, `price_to_intrinsic` pour produits a financement/barriere);
- `Median`;
- `Peers`.

## Pourquoi les premiers resultats etaient faux

Au debut, le calcul comparait:

```text
prix du warrant / valeur intrinseque brute du sous-jacent
```

Cela oubliait que:

- 1 warrant ne represente pas forcement 1 action;
- il faut parfois 10, 50 ou 100 warrants pour une action;
- les produits peuvent etre suspendus;
- certains produits affichent `0`;
- les bid/ask executables ne sont pas forcement disponibles;
- les produits proches du strike peuvent etre domines par valeur temps.

La correction principale a ete:

```text
valeur intrinseque par produit =
  valeur intrinseque brute
  / parite normalisee
  * taux FX vers la devise de cotation
```

Puis classement par pairs plus stricts.

Pour les mini-futures/turbos, une correction supplementaire est appliquee: quand Euronext fournit un deuxieme strike correspondant au niveau de financement, le scoring utilise ce niveau comme reference de payoff. Exemple: `304TB` affiche un seuil de securite autour de `303.93 USD`, mais son niveau de financement est `323.3389 USD`. Utiliser `303.93` donnait une valeur intrinseque beaucoup trop basse et donc un faux signal sur-evalue.

Pour les warrants vanille, une autre correction est appliquee: le ranking ne compare plus `prix / valeur intrinseque`. Exemple: `D12QS` est un Warrant Call Apple strike `280 USD`, maturite `19/03/27`, parite `10`. Avec un spot Apple proche de `300.23 USD`, sa valeur intrinseque convertie en EUR est autour de `1.74 EUR`, mais son prix de marche autour de `3.99 EUR` inclut une valeur temps jusqu'en mars 2027. Le ratio `3.99 / 1.74 = 2.29` n'est donc pas une erreur de marche. La metrique correcte pour ce ranking devient la prime de point mort, environ:

```text
(280 + prix_warrant_USD * 10 - 300.23) / 300.23 * 100
```

Cette valeur est de l'ordre de quelques pourcents, pas de centaines de pourcents.

## Exemple IQ33S

Produit Boursorama:

```text
1rPIQ33S
ISIN: DE000FE5GCW4
Sous-jacent: APPLE
Type: Mini-Future Long / Turbo infini call
Strike 1: 235.89 USD
Strike 2: 228.36 USD
Parite: 50 warrants pour 1 sous-jacent
Maturite: open-end
Emetteur: Societe Generale
```

Dans l'export `analyze`, on retrouve notamment:

- `symbol = IQ33S`;
- `boursorama_symbol = 1rPIQ33S`;
- `isin = DE000FE5GCW4`;
- `strike = 235.8900`;
- `second_strike = 228.3600`;
- `warrants_per_underlying = 50.0000`;
- `leverage = 4.4400`;
- `lower_threshold = 235.8900`;
- `open_price`;
- `close_price`;
- `previous_close_price`;
- `high_price`;
- `low_price`;
- `volume`;
- `quotation_state`;
- `trading_status`;
- `opening_time`;
- `closing_time`;
- `kid_url`.

Avec `BOURSORAMA_ENRICH=1`, on ajoute aussi:

- `boursorama_bid`;
- `boursorama_ask`;
- `boursorama_mid`;
- tailles bid/ask;
- OHLC Boursorama.

## Limites connues

Le projet n'est pas encore un pricer complet.

Limites actuelles:

- bid/ask executables pas toujours disponibles sans enrichissement Boursorama;
- pas encore de frais broker;
- pas encore de verification des conditions emetteur;
- pas encore de volatilite implicite;
- pas encore de modele Black-Scholes pour warrants vanille;
- pas encore de payoff specialise pour toutes les familles exotiques;
- scraping Boursorama couteux si active sur beaucoup de produits;
- certains champs Euronext varient selon emetteur et type de produit.

## Prochaines etapes logiques

1. Ajouter un module `pricing` par famille:
   - `warrant`;
   - `turbo`;
   - `mini_future`;
   - `certificate`.

2. Renforcer le module FX:
   - historiser le taux utilise;
   - detecter les taux obsoletes;
   - ajouter une source de secours si Yahoo ne repond pas.

3. Utiliser prioritairement:
   - mid price si bid/ask disponible;
   - dernier prix sinon;
   - exclure les produits avec spread trop large.

4. Calculer des indicateurs plus solides:
   - spread percent;
   - distance a la barriere;
   - levier effectif;
   - valeur intrinseque corrigee FX;
   - premium/discount;
   - liquidite;
   - score de qualite de donnees.

5. Ajouter une vue Ratatui detaillee par produit:
   - fiche complete;
   - parite;
   - barriere;
   - bid/ask;
   - issuer;
   - horaires;
   - DIC;
   - statut;
   - alertes de donnees manquantes.

6. Ajouter des tests unitaires:
   - call vs put;
   - parite `50` vs `0.02`;
   - produit suspendu;
   - produit sans prix;
   - scoring sous/sur-evalue.

## Principe directeur

La base doit rester modulaire:

```text
collecte donnees -> normalisation -> enrichissement -> filtres qualite -> pricing -> indicateurs -> affichage
```

Chaque etape doit pouvoir evoluer sans casser les autres. C'est ce qui permettra ensuite d'ajouter des indicateurs de plus en plus serieux sans melanger scraping, parsing, logique financiere et interface utilisateur.

## Moteur Black-Scholes, IV et Smile

Le pipeline d'opportunites suit maintenant cette logique:

```text
flux Euronext/Boursorama
  -> data cleansing strict: bid > 0, ask > 0, tailles positives, spread coherent
  -> ajustement FX et dividendes
  -> Black-Scholes sur warrants vanille dates
  -> volatilite implicite du produit
  -> smile median des pairs comparables
  -> signal IV: iv_cheap / iv_neutral / iv_expensive
```

Les variables configurables sont dans `.env`:

```text
OPTION_RISK_FREE_RATE=0.045
OPTION_DIVIDEND_YIELD=0.005
OPTION_IV_SIGNAL_THRESHOLD=0.03
OPPORTUNITY_DEBUG=0
OPPORTUNITY_DEBUG_EVERY=25
MARKET_DATA_API_KEY=changeme_demo_key
DIVIDEND_API_KEY=changeme_demo_key
VOLATILITY_API_KEY=changeme_demo_key
ORATS_BASE_URL=https://api.orats.io
POLYGON_BASE_URL=https://api.polygon.io
```

`OPPORTUNITY_DEBUG=1` active des traces sur stderr pour comprendre ou le pipeline ralentit: recuperation du spot, pages Euronext, enrichissement des details, validation Boursorama, FX, ranking et affichage. `OPPORTUNITY_DEBUG_EVERY` controle la frequence des compteurs dans les boucles longues. Quand le debug est actif et que `OPPORTUNITY_FORMAT` est vide, `manage.sh opportunities` bascule par defaut en sortie `table` pour rendre les traces visibles.

Les trois cles sont fictives pour l'instant. Pour passer en production, il faudra choisir une ou plusieurs APIs:

- taux sans risque: FRED, ECB, Treasury API ou provider broker;
- dividendes/previsions: Polygon, TwelveData, Alpha Vantage, Financial Modeling Prep, Nasdaq Data Link;
- volatilite historique/options chain: Polygon, Tradier, ORATS, Cboe DataShop, Interactive Brokers.

Le modele Black-Scholes actuellement implemente sert aux warrants vanille avec maturite datee. Les turbos, mini-futures et knock-out restent sur leur logique a financement/barriere, car Black-Scholes n'est pas le bon modele principal pour ces produits.

`ORATS_API_KEY` accepte plusieurs tokens separes par des virgules:

```text
ORATS_API_KEY=token_1,token_2,token_3
```

Le client ORATS dedoublonne les tokens en conservant l'ordre. Lors d'un appel ORATS, il essaie le token courant, puis passe automatiquement au suivant en cas de `401`, `403`, `429`, erreur reseau ou erreur serveur `5xx`. Les erreurs fonctionnelles comme `400` ou `404` ne provoquent pas de rotation, car elles signalent plutot une requete invalide ou une ressource absente.

`POLYGON_API_KEY` suit la meme logique:

```text
POLYGON_API_KEY=key_1,key_2,key_3
POLYGON_BASE_URL=https://api.polygon.io
```

Le client Polygon utilise le parametre `apiKey` dans la query string, dedoublonne les cles et passe automatiquement a la suivante en cas de cle refusee, rate limit, erreur reseau ou erreur serveur. La commande de verification rapide est:

```bash
./manage.sh polygon-test RMS.PA
```

Interactive Brokers n'est pas requis par ce pipeline: les chemins ajoutes ici passent uniquement par Polygon, ORATS, Boursorama, Euronext/Yahoo et les donnees FX deja branchees.

Nouveaux champs exportes dans `opportunities-csv` et affiches dans Ratatui:

- `bid_price`, `ask_price`, `mid_price`;
- `spread_pct`;
- `execution_status`;
- `data_quality_score`;
- `liquidity_score`;
- `barrier_distance_pct`;
- `effective_gearing`;
- `premium_discount_pct`;
- `years_to_maturity`;
- `risk_free_rate`;
- `dividend_yield`;
- `implied_volatility`;
- `smile_median_iv`;
- `smile_gap_vol_points`;
- `volatility_signal`.

## Frais broker et rendement net

Le moteur de decision integre maintenant une couche de frais explicite avant de calculer le `R/R`, la taille de position et les stop/targets affiches.

Pipeline:

```text
prix entree ask executable
  -> projection stop/targets bruts
  -> frais achat + depot + vente stop/T1/T2
  -> stop/targets nets de frais
  -> reward/risk net
  -> sizing et decision BUY/WATCH/AVOID
```

Variables `.env`:

```text
BROKER_FEE_PROFILE=custom
FEE_ORDER_NOTIONAL=1000
FEE_BUY_FIXED=
FEE_BUY_PCT=
FEE_SELL_FIXED=
FEE_SELL_PCT=
FEE_DEPOSIT_FIXED=
FEE_DEPOSIT_PCT=
```

Les variables `FEE_*` doivent rester vides pour utiliser les valeurs du profil `BROKER_FEE_PROFILE`. Si `FEE_BUY_FIXED=0` est renseigne explicitement, cela veut dire "forcer le frais d'achat a 0" et cela ecrase donc le profil.

Profils integres:

| Profil | Logique |
| --- | --- |
| `custom` | Aucun frais par defaut; les variables `FEE_*` prennent le relais. |
| `trade_republic` | 1 EUR a l'achat et 1 EUR a la vente; depot SEPA suppose gratuit. |
| `bourse_direct_500` | 0,99 EUR par ordre pour un ordre Euronext <= 500 EUR. |
| `bourse_direct_1000` | 1,90 EUR par ordre pour un ordre Euronext > 500 et <= 1 000 EUR. |
| `bourse_direct_2000` | 2,90 EUR par ordre pour un ordre Euronext > 1 000 et <= 2 000 EUR. |
| `bourse_direct_pct` | 0,09% par ordre pour les ordres > 4 400 EUR. |
| `bourse_direct_morgan_stanley` | 0 EUR par ordre sur la gamme Morgan Stanley warrants/certificats/turbos. |
| `degiro_fr_actions` | 1 EUR de courtage + 1 EUR de frais de gestion par transaction Euronext Paris. |
| `degiro_otc_sg_bnp` | 0,50 EUR par transaction sur produits de bourse OTC Societe Generale / BNP Paribas. |
| `fortuneo_starter` | 0,35% par ordre. |
| `fortuneo_starter_first_500` | 0 EUR par ordre, utile pour simuler le premier ordre mensuel <= 500 EUR. |

Formules:

```text
buy_fee = FEE_BUY_FIXED + notional * FEE_BUY_PCT / 100
deposit_fee = FEE_DEPOSIT_FIXED + notional * FEE_DEPOSIT_PCT / 100
sell_fee(exit) = FEE_SELL_FIXED + exit_notional * FEE_SELL_PCT / 100

cost_basis = notional + buy_fee + deposit_fee
exit_notional = notional * exit_price / entry_price

net_stop_loss_pct = (cost_basis - (stop_notional - sell_stop_fee)) / cost_basis * 100
net_target_gain_pct = ((target_notional - sell_target_fee) - cost_basis) / cost_basis * 100
roundtrip_fee_pct = (buy_fee + deposit_fee + sell_fee) / notional * 100
```

Dans Ratatui:

- colonne `Fee`: cout aller-retour estime jusqu'a T2 en pourcentage du montant d'ordre;
- ligne `Fees`: profil, montant simule, frais achat/depot/vente et transformation brut -> net;
- ligne `Calc fees`: formule numerique appliquee au stop et a T2.

Attention: ces frais broker ne remplacent pas le spread deja mesure dans le carnet, ni les frais implicites de financement/produit inclus par l'emetteur dans le prix du warrant/turbo. Ils servent a comparer le cout de passage d'ordre entre plateformes et a eviter qu'un edge de 1-2% soit transforme en faux signal par des frais fixes trop lourds sur un petit ordre.

## Mode scenario: intention humaine et prevoyance

La commande `scenario` sert a tester une these explicite:

```text
"Je vise RMS.PA a 1800 EUR le 2026-10-31 et je veux des warrants call
avec une maturite debut 2027."
```

Commande type:

```bash
BROKER_FEE_PROFILE=bourse_direct_1000 FEE_ORDER_NOTIONAL=1000 \
CANDLES_RANGE=60d CANDLES_INTERVAL=60m MARKET_DATA_REFRESH=cache \
./manage.sh scenario hermes RMS.PA 1800 2026-10-31 2027-01-01 2027-03-31 call 500
```

Si le terminal est interactif, `manage.sh scenario` ouvre maintenant une vue Ratatui par defaut. Pour une sortie scriptable:

```bash
SCENARIO_FORMAT=table ./manage.sh scenario hermes RMS.PA 1800 2026-10-31 2027-01-01 2027-03-31 call 500
```

Remplace `call` par `auto` pour laisser le moteur comparer calls et puts; garde `call` ou `put` uniquement pour forcer un cote.

Logique:

```text
produits scenario nettoyes
  -> mode auto: conserve calls et puts, sauf filtre explicite call/put
  -> filtre maturite apres la date cible
  -> filtre fenetre de maturite voulue
  -> prix d'entree = ask executable
  -> projection Black-Scholes ou lineaire selon le modele produit
  -> application des frais broker
  -> calcul rendement net, point mort sous-jacent, score intentionnel
```

En mode `SCENARIO_SIDE=auto`, le type nominal du produit (`call` ou `put`) sert au pricing du produit, mais la probabilite `Tch<=D` suit la direction de la these de marche: cible au-dessus du spot = first-touch haussier, cible sous le spot = first-touch baissier. Cela evite le faux 100% qui apparaitrait si un call etait evalue sur une cible situee sous le spot.

Modeles scenario actuellement supportes:

| Famille / modele | Projection | Theta / Vega |
| --- | --- | --- |
| `warrant / warrant_intrinsic` | Black-Scholes a la date cible avec IV, FX, dividendes et spread de sortie | Oui |
| `turbo / financing_level` | `max(direction * (S_target - financement), 0) / parite * fx` | Non |
| `mini_future / financing_level` | meme projection lineaire | Non |
| `open_end_knock_out / financing_level` | projection lineaire, maturite affichee `open-end` | Non |
| `open_end_knock_out / barrier_only` | projection lineaire sur reference/barriere disponible | Non |

Les certificats a payoff conditionnel (`discount`, `bonus`, `express`, leverage constant, etc.) restent exclus du mode scenario tant que leur payoff contractuel n'est pas modelise.

Pour les produits lineaires open-end sans taux de financement renseigne, le niveau de financement/barriere futur n'est pas projete. Le moteur ajoute donc `future_financing_not_projected`. Si la cible est a plus de `SCENARIO_MAX_UNMODELED_LINEAR_BUY_DAYS` jours, il ajoute aussi `linear_financing_unmodeled_long_horizon`, ce qui bloque `BUY` mais garde le produit visible en `WATCH` si les autres conditions sont positives.

Si `SCENARIO_LINEAR_FINANCING_RATE_PCT` est renseigne, la reference de financement/barriere open-end est projetee:

```text
call reference_future = reference_now * exp(financing_rate * T)
put  reference_future = reference_now * exp(-financing_rate * T)
```

Cela modelise l'erosion economiquement defavorable aux deux sens: le niveau monte pour un long/call et baisse pour un short/put. Le champ `financing_drag_pct` mesure l'impact sur le prix de sortie par rapport a une reference non projetee.

Champs principaux:

| Champ | Sens |
| --- | --- |
| `targetPx` | Prix produit estime a la date cible si le sous-jacent atteint le prix cible. |
| `net%` | Rendement estime net de frais broker. |
| `gross%` | Rendement brut avant frais. |
| `BE` | Cours sous-jacent a atteindre a la date cible pour etre a l'equilibre net de frais. |
| `fee%` | Cout total achat/depot/vente en pourcentage du montant d'ordre. |
| `iv%` | Volatilite implicite actuelle quand elle est disponible. |
| `score` | Score de coherence: rendement net, qualite de donnees, liquidite, spread, IV relative et fit de maturite. |
| `KO%` | Probabilite first-touch de barriere avant la date cible. |
| `MCev` | Esperance de rendement de la simulation Monte Carlo target/stop/KO. |

Indicateurs mathematiques et statistiques ajoutes:

| Champ | Formule | Lecture |
| --- | --- | --- |
| `PBE` | Probabilite lognormale que le sous-jacent atteigne le breakeven a la date cible. | Plus c'est haut, plus le point mort est statistiquement proche. |
| `PTgt` | Probabilite lognormale que le sous-jacent atteigne le prix cible a la date cible. | Mesure la difficulte de la these selon vol et horizon. |
| `zTarget` | `z=(ln(target/spot)-(r-q-0.5*sigma^2)T)/(sigma*sqrt(T))`. | Nombre d'ecarts-types lognormaux a franchir. |
| `zBE` | Meme formule avec `level=breakeven`. | Effort statistique minimal pour ne pas perdre. |
| `EV` | Valeur attendue risk-neutral du warrant a l'horizon cible, nette de frais, comparee au prix d'entree. | Detecte si le prix d'entree est cher/pas cher selon le modele, sans supposer que la cible arrive. |
| `Sharpe-like` | `E[R] / std(R)` sur un modele binaire cible atteinte vs perte totale. | Indicateur simple rendement/risque, volontairement conservateur. |
| `Kelly` | `max(0, (b*p-q)/b)` avec `b=gain/perte`, `p=P(target)`, `q=1-p`. | Taille theorique agressive; a lire comme plafond statistique, pas comme consigne. |
| `Delta` | Delta Black-Scholes converti par `fx/parity`. | Variation theorique du warrant pour +1 unite de sous-jacent. |
| `Gamma` | Gamma Black-Scholes converti par `fx/parity`. | Acceleration du delta. |
| `Vega/pt` | Vega Black-Scholes pour +1 point de volatilite, converti par `fx/parity`. | Sensibilite a la volatilite implicite. |
| `Theta/j` | Theta Black-Scholes annuel / 365, converti par `fx/parity`. | Cout temps quotidien theorique. |
| `Rho/1%` | Rho Black-Scholes pour +1 point de taux, converti par `fx/parity`. | Sensibilite aux taux. |

References de formule:

- Black-Scholes: prix, `d1`, `d2` et sensibilites derivees du modele d'option europeenne.
- NIST Normal Distribution: densite normale, fonction cumulative `Phi`, z-score.
- William F. Sharpe: ratio rendement/variabilite.
- J. L. Kelly: maximisation de la croissance logarithmique, fraction de capital.

Monte Carlo:

```text
S_{t+dt} = S_t * exp((mu - 0.5*sigma^2)*dt + sigma*sqrt(dt)*Z)
```

Le moteur utilise le drift reel configure (`SCENARIO_REAL_DRIFT_PCT`) pour les trajectoires. A chaque pas il revalorise le produit, teste le target, le stop et la barriere. Les sorties sont:

- `target_first_pct`
- `stop_first_pct`
- `ko_pct`
- `monte_carlo_expected_return_pct`
- `p05/p50/p95` du P/L

Decision:

- `BUY`: le scenario est positif, liquide, executable, avec rendement net suffisant;
- `WATCH`: le scenario est interessant mais fragile ou incomplet;
- `AVOID`: rendement negatif, liquidite trop basse, spread trop large, point mort au-dela de la cible, ou donnees insuffisantes.

Garde-fous supplementaires:

- `expected_value_negative`: EV risk-neutral negative; bloque `BUY`, mais peut rester `WATCH` si le payoff conditionnel au target est positif;
- `monte_carlo_ev_negative`: EV Monte Carlo negative; bloque `BUY`, mais peut rester `WATCH`;
- `target_before_stop_not_favored`: le stop/KO arrive au moins aussi souvent que le target;
- `barrier_touch_probability_too_high`: probabilite KO superieure a `SCENARIO_MAX_BUY_KO_PROB_PCT`;
- `linear_financing_unmodeled_long_horizon`: produit lineaire open-end sur horizon long; bloque `BUY` tant que le financement futur n'est pas projete;
- `SCENARIO_MAX_UNMODELED_LINEAR_BUY_DAYS`: seuil de jours pour ce garde-fou, `45` par defaut.

Ce mode ne predit pas que le sous-jacent ira au prix cible. Il repond seulement a la question: "si mon intention de marche se realise, quel warrant exprime le mieux cette idee compte tenu du prix, de la maturite, de l'IV, de la liquidite et des frais ?"

### Audit LLM Gemini

Dans la vue Ratatui du mode `scenario`, l'onglet LLM ajoute une lecture qualitative par Gemini. Le LLM ne remplace pas les calculs: il audite le signal, cherche les incoherences, liste les risques et propose un verdict prudent.

Touches:

- `l`: bascule Notes / LLM Gemini.
- `r`: interroge Gemini pour le warrant selectionne.
- `Tab`, `Shift+Tab`, `←`, `→`, `n`, `p`, `t`, `e`: scroll dans l'onglet actif.

Variables `.env`:

```env
LLM_ENABLE=1
GEMINI_API_KEY=cle_1,cle_2
GEMINI_MODEL=gemini-3-pro-preview
GEMINI_BASE_URL=https://generativelanguage.googleapis.com
LLM_CACHE=1
LLM_CACHE_DIR=data/cache/llm
```

`GEMINI_API_KEY` accepte plusieurs cles separees par des virgules. En cas de `401`, `403`, `429`, erreur reseau ou erreur serveur `5xx`, le client essaie la cle suivante.

Choix de modele:

- `gemini-3-pro-preview`: modele haut de gamme actuel pour un audit pousse, mais en preview.
- `gemini-2.5-pro`: option plus stable.
- `gemini-2.5-flash`: option plus rapide et moins couteuse.

Le contexte envoye est volontairement structure: sous-jacent, scenario, warrant, prix d'entree, projection, frais, probabilites first-touch et terminales, greeks actuels et projetes, stress IV/FX, dividendes discrets, qualite de donnees, flow informatif, bid/ask, spread et raisons du verdict quantitatif. La reponse demandee est un JSON strict pour rester affichable proprement.

La reponse LLM est decoupee en sections directement utiles dans l'interface: drivers du verdict, red flags, risques principaux, checks d'execution, problemes de donnees, revue du plan, conditions d'invalidation et questions avant entree. Le flow/volume observe ne doit jamais etre utilise comme raison de decision, car l'execution de ces produits depend surtout du market maker, du bid/ask executable, du spread, des tailles affichees, du statut et de la fraicheur des donnees.

Implementation:

- les regles de comportement sont envoyees dans `systemInstruction`;
- le prompt utilisateur contient le JSON de contexte et le schema attendu;
- `responseSchema` force une reponse JSON structuree;
- le cache LLM utilise un SHA-256 stable sur le modele, le system prompt et le prompt utilisateur;
- si Gemini renvoie `promptFeedback.blockReason`, l'interface affiche une erreur explicite au lieu d'un message generique.
