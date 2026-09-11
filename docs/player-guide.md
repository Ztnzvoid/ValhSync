# ValhSync player guide

*Version française plus bas.*

## Install

There is nothing to install, nothing to open on your router, and no
administrator rights needed. ValhSync only makes outgoing connections, like a
browser, and only writes inside the Valheim folder.

Put `valhsync.exe` anywhere (your Desktop is
fine). If your admin gave you a zip, keep `valhsync-invite.txt` next to the
executable.

## First start

1. Double-click `valhsync.exe`.
2. If your server is not listed, click **Add a server** and paste the invite
   code your admin sent you (it starts with `valhsync1:`).
3. The launcher contacts the server and shows what it will do: files to
   install, to update, and any unknown mod files it will move to a quarantine
   folder (never deleted).
4. Press **PLAY**. Valheim starts through Steam and connects to the server.

Close Valheim before syncing; the launcher refuses to change files while the
game runs.

## Every other time

Press **PLAY**. If the admin updated a mod, the launcher downloads only what
changed. If nothing changed, it just starts the game.

## The other buttons

- **Go back to the previous version**: undoes the last sync exactly, from the
  backup taken before it. Useful if a mod update breaks something and you want
  to play elsewhere while the admin fixes it.
- **Open quarantine**: shows the folder where unknown mod files were moved
  (`BepInEx/_valhsync_quarantine/` inside the game folder). Nothing there is
  deleted by ValhSync.
- **Play without mods**: disables BepInEx (renames `winhttp.dll`) and starts the
  vanilla game. The next PLAY re-enables it.
- **Settings**: set the game folder by hand if Steam detection fails (the
  folder containing `valheim.exe`), forget a server, switch language.

Your own changes to config files (keybinds, UI positions) survive syncs: the
server seeds configs but does not overwrite them, unless the admin enforces a
specific file.

## Linux and Steam Deck

The sync works the same. BepInEx itself only loads if Valheim's launch options
are set in Steam (Valheim > Properties > Launch options):

- Proton: `WINEDLLOVERRIDES="winhttp=n,b" %command%`
- Native Linux build: `./start_game_bepinex.sh %command%`

The launcher shows the right line for your install.

## Command line

The same executable works in a terminal:

```
valhsync join <code>      add a server
valhsync status           show what a sync would do
valhsync sync             sync without starting the game
valhsync play             sync and start the game
valhsync rollback         undo the last sync
valhsync vanilla on|off   toggle BepInEx
valhsync doctor           show what ValhSync detected on this machine
```

## When something goes wrong

Every error message says what happened and what to do. The common ones:

- *cannot reach the server*: the admin's `valhsync-server` is not running, or
  the port is closed. Tell the admin.
- *key does not match*: the server's key changed. Do not accept a new code
  from anywhere but the admin directly; then `Add a server` again.
- *Valheim is running*: close the game first.
- *Valheim was not found*: set the folder in Settings.

ValhSync keeps its state in `%APPDATA%\valhsync` and its backups in
`%LOCALAPPDATA%\valhsync\backups` (Linux: `~/.config/valhsync`,
`~/.local/share/valhsync/backups`). Deleting those folders resets the launcher
without touching the game.

---

## En français

### Installation

Rien à installer, rien à ouvrir sur ta box, pas de droits administrateur.
ValhSync ne fait que des connexions sortantes, comme un navigateur, et n'écrit
que dans le dossier de Valheim.

Mets `valhsync.exe` où tu veux. Si ton admin t'a donné un
zip, garde `valhsync-invite.txt` à côté de l'exécutable.

### Premier lancement

1. Double-clic sur `valhsync.exe`.
2. Si ton serveur n'apparaît pas, clique **Ajouter un serveur** et colle le
   code d'invitation reçu de l'admin (il commence par `valhsync1:`).
3. Le launcher interroge le serveur et affiche ce qu'il va faire : fichiers à
   installer, à mettre à jour, et les fichiers de mods inconnus qu'il va
   déplacer en quarantaine (jamais supprimés).
4. Bouton **JOUER**. Valheim démarre via Steam et se connecte au serveur.

Ferme Valheim avant : le launcher refuse de toucher aux fichiers pendant que
le jeu tourne.

### Les autres fois

**JOUER**. Si l'admin a mis un mod à jour, seul ce qui a changé est
téléchargé. Sinon, le jeu démarre directement.

### Les autres boutons

- **Revenir à la version précédente** : annule exactement la dernière synchro.
- **Voir la quarantaine** : ouvre le dossier où les fichiers inconnus ont été
  déplacés (`BepInEx/_valhsync_quarantine/` dans le dossier du jeu).
- **Jouer sans mods** : désactive BepInEx et lance le jeu vanilla. Le prochain
  JOUER le réactive.
- **Réglages** : dossier du jeu à la main si Steam n'est pas détecté, oublier
  un serveur, langue.

Tes réglages dans les fichiers de config (raccourcis, interface) survivent aux
synchros.

### Linux et Steam Deck

Même synchro, mais BepInEx ne se charge que si les options de lancement Steam
sont réglées : `WINEDLLOVERRIDES="winhttp=n,b" %command%` sous Proton,
`./start_game_bepinex.sh %command%` pour la version Linux native. Le launcher
affiche la bonne ligne.

### En cas de problème

Chaque message d'erreur dit ce qui s'est passé et quoi faire. *Serveur
injoignable* : le serveur ValhSync de l'admin ne tourne pas ou le port est
fermé. *La clé ne correspond pas* : n'accepte un nouveau code que de l'admin
directement. *Valheim est lancé* : ferme le jeu. *Valheim introuvable* :
indique le dossier dans Réglages.
