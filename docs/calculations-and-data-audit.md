# Calculs et Audit des Donnees

Ce document decrit les calculs utilises par l'application et les controles appliques aux donnees recues. Il sert de reference pour verifier qu'un changement de code ne casse pas silencieusement le moteur quantitatif.

Les resultats restent des estimations. Un warrant, un turbo ou un mini-future peut perdre 100% de sa valeur; l'emetteur peut aussi modifier ses conditions de cotation.

## Sources Controlees

### Catalogue Produits

Le mode `discover` et le mode `scenario` recuperent les produits depuis Euronext/Boursorama. Les champs critiques sont:

- `symbol` / ISIN: identifiant du produit.
- `product_type`: famille du produit, par exemple warrant, turbo, mini-future, knock-out.
- `strike` ou niveau de financement: niveau de reference du payoff.
- `maturity`: date de maturite ou `open-end`.
- `bid_ask`: prix executable si le bid et l'ask sont presents et positifs.
- `parity`: nombre de produits pour une unite de sous-jacent.

Les pages Boursorama publiques de type palmares sont optionnelles (`BOURSORAMA_DISCOVER_PUBLIC_PAGES=1`) car leur table n'a pas toujours les memes colonnes que les pages produit. Par defaut, le moteur privilegie les endpoints dont le schema est stable.

### Prix Executable

Le prix d'entree utilise l'ask quand le carnet est exploitable:

```text
ask > 0
bid > 0
ask >= bid
spread relatif <= seuil executable
tailles bid/ask non nulles si disponibles
ecart ask/last non aberrant si last disponible
```

Si l'ask vaut zero, le produit est considere non executable. Le moteur ne retombe pas sur `last_price`, car un dernier prix sans ask peut etre stale et impossible a acheter.

### Qualite de Donnees

Le score de qualite part de 100 pour un bid/ask executable. Il penalise:

- spread absent;
- spread large;
- volume explicitement nul;
- prix `last_unverified`;
- statut non traite ou donnees detaillees absentes.

Le champ `Flow` est informatif pour les produits OTC: il ne doit pas condamner seul un produit, mais le bid/ask reste obligatoire.

### FX

Les taux de change sont stockes dans `FxRateBook`. Le moteur accepte:

- conversion directe, par exemple USD -> EUR;
- conversion inverse, par exemple EUR -> USD;
- identite, par exemple EUR -> EUR;
- codes devises insensibles a la casse.

Pour un sous-jacent USD et un produit EUR, la valorisation convertit le prix theorique du sous-jacent vers la devise de cotation du produit.

## Formules Options et Warrants

### Black-Scholes avec Dividendes Continus

Pour les warrants vanilla, le moteur utilise Black-Scholes/Merton:

```text
d1 = [ln(S/K) + (r - q + 0.5 sigma^2) T] / [sigma sqrt(T)]
d2 = d1 - sigma sqrt(T)

Call = S exp(-qT) Phi(d1) - K exp(-rT) Phi(d2)
Put  = K exp(-rT) Phi(-d2) - S exp(-qT) Phi(-d1)
```

Variables:

- `S`: spot ou spot projete.
- `K`: strike.
- `T`: temps restant jusqu'a maturite.
- `r`: taux sans risque.
- `q`: rendement de dividende continu, ou zero si les dividendes discrets sont modelises separement.
- `sigma`: volatilite implicite ou volatilite de repli.

Le prix warrant par produit est ensuite:

```text
prix_produit = prix_option_par_action / parite * FX
```

### Fair-Value Risk-Neutral

Le champ historique `expected_value_pct` ne doit pas etre lu comme une esperance de gain monde reel. Il mesure plutot un ecart de fair-value sous la mesure risque-neutre:

```text
fair_now = prix modele aujourd'hui
expected_exit_Q = fair_now * exp(r * T_target)
fair_value_Q_pct = rendement net de expected_exit_Q vs prix d'entree
```

Cette valeur sert a detecter un produit cher ou bon marche par rapport au modele. La vraie esperance de trajectoire est `MCev`, issue du Monte Carlo avec target, stop mark-to-market et KO.

### Parite Call/Put

Invariant teste:

```text
Call - Put = S exp(-qT) - K exp(-rT)
```

Ce test protege contre une erreur de signe sur `q`, `r`, `d1` ou `d2`.

### Grecs

Les grecs affiches sont ceux du modele Black-Scholes, ramenes a l'unite lisible par produit:

- `delta`: variation du prix theorique pour une variation de 1 unite du sous-jacent.
- `gamma`: convexite du delta.
- `vega/pt`: variation pour 1 point de volatilite, pas pour 100%.
- `theta/j`: erosion theorique par jour.
- `rho/1%`: variation pour 1 point de taux.

Les tests comparent delta, gamma et vega a des differences finies autour du prix Black-Scholes.

### Volatilite Implicite

L'IV est resolue par dichotomie. Le moteur refuse une IV quand:

- le prix marche est non positif;
- le spot, strike ou temps sont invalides;
- le prix marche est inferieur a l'intrinseque actualise;
- meme une volatilite tres haute ne permet pas d'atteindre le prix observe.

### Probabilites

La probabilite terminale utilise une loi lognormale:

```text
z = [ln(level/S) - (drift - 0.5 sigma^2) T] / [sigma sqrt(T)]

Call: P(S_T >= level) = 1 - Phi(z)
Put:  P(S_T <= level) = Phi(z)
```

La probabilite `Tch<=D` est une probabilite de premier passage avant la date cible. Elle est plus adaptee a un take-profit qu'une probabilite terminale pure. Elle reste un modele, pas une prediction.

## Produits Lineaires

Les turbos, mini-futures et knock-out compatibles sont projetes avec un payoff lineaire:

```text
Call/Long: intrinsic = max(S_eff - niveau_reference, 0)
Put/Short: intrinsic = max(niveau_reference - S_eff, 0)
prix = intrinsic / parite * FX
```

Ces produits ne sont pas prices avec Black-Scholes. Donc:

- pas de theta Black-Scholes;
- pas de vega Black-Scholes;
- risque principal: financement futur, barriere/knock-out, spread et FX.

Pour les produits open-end, le niveau de financement futur peut glisser. Le moteur doit donc documenter le risque au lieu de promettre une precision parfaite.

Le levier historique `effective_gearing` est un levier simple:

```text
simple_gearing = S / (prix_produit * parite)
```

Ce n'est pas l'omega d'un warrant vanilla (`delta * S / V`). Il est utile pour les produits lineaires, mais peut surestimer la sensibilite d'un warrant hors de la monnaie.

## Frais et Rendement Net

Le cout d'entree est:

```text
cost_basis = montant_ordre + frais_achat + frais_depot
```

Le montant de sortie est proportionnel au prix de sortie:

```text
exit_notional = montant_ordre * exit_price / entry_price
```

Le rendement net est:

```text
net_return = [(exit_notional - frais_vente) - cost_basis] / cost_basis
```

Les tests verifient les frais fixes, les frais en pourcentage et leur combinaison avec des calculs manuels.

## Spread de Sortie

Le prix `ExitBid@D` est volontairement prudent: il represente une sortie estimee au bid, pas un mid theorique ideal. Le moteur applique une penalite de spread afin de ne pas surestimer la revente.

## Target Range

Quand le target est une zone, par exemple `365..375`, le moteur calcule:

- bas de range;
- milieu de range;
- haut de range.

Par defaut, la decision reste basee sur le milieu pour conserver le comportement du target unique. Les notes affichent min/moy/max. Les modes `SCENARIO_RANGE_DECISION_MODE=avg` et `worst` permettent de rendre la range plus stricte.

## Monte Carlo

Le Monte Carlo simule des chemins GBM avec graine deterministe pour tester:

- probabilite d'atteindre le target;
- probabilite d'atteindre le stop;
- risque knock-out;
- esperance conditionnelle du plan.

Le stop Monte Carlo est un stop mark-to-market en pourcentage de perte produit (`max_loss_pct_per_trade`). Il ne represente pas exactement le stop sous-jacent ATR/support du `TradePlan`. Il complete les formules fermees, mais ne remplace pas les donnees de marche.

## Fallback Sans IV

Quand un produit n'a ni IV exploitable ni maturite exploitable, le fallback conserve la valeur temps actuelle:

```text
prix_projete = intrinseque_projete + valeur_temps_actuelle
```

C'est une hypothese forte. Elle evite de jeter tout le produit, mais elle doit etre lue comme une approximation de secours, pas comme un pricing canonique.

## Batteries de Tests

| Zone | Tests |
| --- | --- |
| Black-Scholes | Prix de reference, parite call/put, rejet des inputs invalides |
| Grecs | Signes, echelles, differences finies delta/gamma/vega |
| IV | Recuperation d'une volatilite connue, bornes intrinseques |
| Probabilites | Direction call/put, first-touch > terminal, reflection principle |
| Frais | Frais fixes, pourcentage, depot, cashflows manuels |
| FX | Identite, direct, inverse, casse des devises |
| Donnees Boursorama | Zero ask refuse, zero bid refuse, bid/ask inverse refuse, spread extreme refuse |
| Produits | Warrant vanilla, mini-future, turbo/KO, open-end |
| Scenario | Fees, spread, FX, dividendes, range target, auto-side, barrier risk |
| LLM | JSON structure, cache stable, erreurs safety Gemini |

## Limites Connues

- Le modele Black-Scholes suppose une volatilite continue et une execution propre; les warrants OTC peuvent coter autrement.
- Les dividendes discrets ameliorent le modele mais restent des estimations.
- Le drift risque-neutre n'est pas une probabilite monde reel.
- Le spread de l'emetteur peut s'elargir fortement quand le marche bouge.
- Les produits open-end dependent du financement futur; sans grille emetteur complete, le calcul reste une approximation.
- Une page Boursorama peut changer de structure. Les tests figent nos parseurs, mais ils ne garantissent pas que la source ne changera jamais.
