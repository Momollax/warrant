# Warrant Fetcher

Outil Rust/Docker pour recuperer et analyser des produits structures lies a un sous-jacent comme Hermes.

La documentation principale est ici:

- [Documentation technique et logique](docs/technical-and-logic.md)

Commandes utiles:

```bash
./manage.sh build
./manage.sh discover hermes 50
./manage.sh analyze hermes RMS.PA 50
./manage.sh opportunities hermes RMS.PA 500
./manage.sh opportunities hermes RMS.PA 500 call
./manage.sh opportunities-csv hermes RMS.PA 500 put
```

Enrichissement optionnel Boursorama:

```bash
BOURSORAMA_ENRICH=1 ./manage.sh analyze hermes RMS.PA 50
```

Les signaux produits sont des indicateurs d'analyse, pas des recommandations d'investissement.
