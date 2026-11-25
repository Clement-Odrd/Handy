Read @AGENTS.md

# Fork Clement-Odrd/Handy : dictée avec Mistral Voxtral

Ce dépôt est un fork de [cjpais/Handy](https://github.com/cjpais/Handy) (MIT). Les règles d'`AGENTS.md` s'appliquent toutes ; cette section ne décrit que ce qui est propre au fork.

## Objectif

Une app de dictée système, dans l'esprit de Wispr Flow : un raccourci global, une bulle d'aperçu pendant la dictée, puis le texte final collé dans le champ actif. Elle offre une transcription cloud européenne (Mistral Voxtral) ou locale (Parakeet), et elle tient compte du vocabulaire métier. Usage personnel d'abord, produit commercialisable ensuite.

## Remotes et branches

- `origin` = `github.com/Clement-Odrd/Handy` (ce fork, **public** : aucune donnée client ni information commerciale dans le code, les commits ou la doc).
- `upstream` = `github.com/cjpais/Handy`. Récupérer l'amont avec `git fetch upstream && git rebase upstream/main` sur une branche dédiée. Ne jamais ouvrir de PR vers l'amont sans décision explicite.
- Une branche par lot (`feat/<sujet>`). Isoler les ajouts du fork dans des modules à part (`src-tauri/src/cloud/`, `commands/cloud.rs`) pour limiter les conflits avec l'amont.

## Feuille de route

| Lot     | Contenu                                                                                                                                                                   | État                         |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------- |
| 1       | Moteur Voxtral en différé : clé dans le trousseau, `custom_words` envoyés en `context_bias`                                                                               | Fait (`feat/voxtral-engine`) |
| 2       | Double passe locale : aperçu en direct avec Nemotron Streaming 3.5, texte final avec Parakeet Ultra au relâchement                                                        | À faire                      |
| 3       | Profils de contexte : mots personnalisés et prompt de post-traitement par métier ou par client, sélection depuis la barre de menus                                        | À faire                      |
| 4       | Mise en forme dictée : commandes vocales (gras, titres, retour à la ligne) et LLM (listes), puis Markdown converti en collage HTML et texte via `arboard` `set_html`      | À faire                      |
| 5       | Bascule automatique de Voxtral vers le modèle local quand le réseau manque, et coupure du vérificateur de mises à jour                                                    | À faire                      |
| Produit | Changer le nom, le logo et l'identifiant `com.pais.handy` ; proxy serveur pour la clé API ; signature et notarisation ; audit des licences des dépendances et des modèles | Plus tard                    |

## Lot 1 : repères dans le code

- `src-tauri/src/cloud/voxtral.rs` : client Voxtral (WAV construit en mémoire, multipart, HTTPS uniquement, redirections refusées, délais de 10 s à la connexion et 180 s au total, annulation). Le modèle est `voxtral-mini-transcribe-3`.
- `src-tauri/src/cloud/keychain.rs` : clé API dans le trousseau du système (crate `keyring`). Elle n'est jamais écrite dans le store, jamais journalisée, jamais renvoyée au frontend.
- `src-tauri/src/commands/cloud.rs` : commandes Tauri (moteur choisi, enregistrement et suppression de la clé, présence de la clé).
- `src-tauri/src/managers/transcription.rs` : bascule dans `transcribe_recording`, appel dans `decode_voxtral`. La correction floue des `custom_words` reste appliquée après Voxtral, car Mistral annonce le `context_bias` comme expérimental en français.
- `src-tauri/src/actions.rs` : avec Voxtral, pas de chargement de modèle local, et la clé est vérifiée avant d'ouvrir le micro.
- `src/components/settings/models/TranscriptionProvider.tsx` : choix du moteur et champ de clé en écriture seule.
- Limites connues : la retranscription depuis l'historique et l'onboarding exigent encore un modèle local ; l'option de traduction vers l'anglais est ignorée avec Voxtral ; un client HTTP est créé à chaque dictée.

## Règles de sécurité propres au fork

- Toute clé API passe par le trousseau du système, jamais par `SecretMap` ni par le store en clair.
- Aucun fichier audio n'est écrit sur disque pour un appel cloud.
- Les tests n'appellent jamais une API réelle : passer par un serveur local (voir les tests de `cloud::`).

## Environnement de build (macOS)

- PATH nécessaire : `export PATH="$HOME/Library/Python/3.14/bin:$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"`. `cmake` est installé avec `pip install --user cmake`, car Homebrew est absent.
- **Piège de Rust 1.100** : la nouvelle disposition du dossier de build fait copier `resources/` dans `target/debug/build/` au lieu de `target/debug/`. `bun tauri dev` plante alors au démarrage (`lib.rs:262`, icône du tray introuvable). Contournement local, à refaire après un `cargo clean` : `ln -sfn build/resources src-tauri/target/debug/resources`. Le bundle de release n'est pas touché.
- Construire l'app (sans clé de signature des mises à jour) : `bun run tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'`. Le résultat est dans `src-tauri/target/release/bundle/macos/Handy.app`.
- Le vérificateur de mises à jour pointe encore vers les releases de l'amont : accepter une mise à jour remplace le fork par la version officielle.

## Conventions de commit

Conventional commits (`type(scope): sujet`), avec le pourquoi dans le corps du message. Aucune mention d'IA ni ligne `Co-Authored-By` dans les commits ou les PR.
