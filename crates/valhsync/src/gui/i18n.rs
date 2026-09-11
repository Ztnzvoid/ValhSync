//! Two languages, one enum. French first because that is who plays on the
//! server this was built for; English because the project is public.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    /// English unless the player chose French: the project is public, and the
    /// other language is one click away in the header.
    pub fn detect(setting: Option<&str>) -> Self {
        match setting.map(str::to_lowercase) {
            Some(code) if code.starts_with("fr") => Self::Fr,
            _ => Self::En,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Fr => "fr",
            Self::En => "en",
        }
    }

    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Fr => Self::En,
            Self::En => Self::Fr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Subtitle,
    AddServer,
    NoServerHint,
    Checking,
    GameUp,
    GameDown,
    UpToDate,
    FirstSync,
    Pending,
    Play,
    Update,
    Launching2,
    Repair,
    RepairHint,
    ForgetServer,
    SetAsideCount,
    Rollback,
    SetAside,
    SetAsideHint,
    SetAsideNone,
    GameFolder,
    GameFolderHint,
    GameFolderInUse,
    GameFolderAuto,
    ThisServer,
    AboutValhSync,
    OpenFolder,
    ResetAll,
    ResetAllHint,
    ResetDone,
    Apply,
    Cancel,
    Close,
    InvitePrompt,
    InviteHint,
    JoinCodeNotAnAddress,
    Checking2,
    ConfirmKeyTitle,
    ConfirmKeyBody,
    Trust,
    Added,
    Launching,
    SyncDone,
    RolledBack,
    Downloading,
    Applying,
    Contacting,
    PlanInstall,
    PlanUpdate,
    PlanRemove,
    PlanQuarantine,
    PlanDownload,
    ConfirmTitle,
    ConfirmBody,
    Forget,
    Refresh,
    Settings,
    Language,
    Busy,
    KeyFingerprint,
    TrustLine,
    Installed,
    Nothing,
    ServerMods,
    ShowAll,
    ModToInstall,
    ModToUpdate,
    ModInstalled,
    Speed,
    Remaining,
}

pub fn text(lang: Lang, key: Key) -> &'static str {
    match lang {
        Lang::Fr => fr(key),
        Lang::En => en(key),
    }
}

fn fr(key: Key) -> &'static str {
    match key {
        Key::Subtitle => "Vos mods, alignés sur le serveur. Puis on joue.",
        Key::AddServer => "Ajouter un serveur",
        Key::NoServerHint => {
            "Collez le code d'invitation que l'admin vous a envoyé, ou placez son fichier valhsync-invite.txt à côté de valhsync.exe."
        }
        Key::Checking => "Vérification auprès du serveur…",
        Key::GameUp => "Serveur de jeu en ligne",
        Key::GameDown => "Serveur de jeu arrêté — vous ne pourrez pas vous connecter",
        Key::UpToDate => "À jour",
        Key::FirstSync => "Première synchronisation",
        Key::Pending => "Mise à jour disponible",
        Key::Play => "JOUER",
        Key::Update => "METTRE À JOUR",
        Key::Launching2 => "Démarrage de Valheim",
        Key::Repair => "Réparer",
        Key::RepairHint => {
            "Revérifie chaque fichier et remet en place tout ce qui diffère du serveur, y compris vos configurations."
        }
        Key::ForgetServer => "Retirer ce serveur de la liste",
        Key::SetAsideCount => "mods mis de côté",
        Key::Rollback => "Revenir à la version précédente",
        Key::SetAside => "Mods mis de côté",
        Key::SetAsideHint => {
            "Fichiers de mods trouvés chez vous mais absents du pack du serveur : ValhSync les a déplacés pour qu'ils ne bloquent pas la connexion. Rien n'est supprimé, vous pouvez les récupérer."
        }
        Key::SetAsideNone => "Aucun mod mis de côté.",
        Key::GameFolder => "Dossier du jeu",
        Key::GameFolderHint => {
            "Laissez vide pour que ValhSync trouve Valheim tout seul via Steam. Remplissez seulement si la détection échoue."
        }
        Key::GameFolderInUse => "Utilisé actuellement",
        Key::GameFolderAuto => "Détection automatique",
        Key::ThisServer => "Ce serveur",
        Key::AboutValhSync => "ValhSync",
        Key::OpenFolder => "Ouvrir le dossier",
        Key::ResetAll => "Tout réinitialiser",
        Key::ResetAllHint => {
            "Oublie tous les serveurs et les réglages de ValhSync. Ne touche pas au jeu ni aux mods installés."
        }
        Key::ResetDone => "ValhSync réinitialisé.",
        Key::Apply => "Appliquer",
        Key::Cancel => "Annuler",
        Key::Close => "Fermer",
        Key::InvitePrompt => {
            "Code d'invitation, ou simplement l'adresse du serveur de jeu — la même que dans Valheim"
        }
        Key::InviteHint => "valhsync1:…  ou  monserveur.exemple.org:2456",
        Key::JoinCodeNotAnAddress => {
            "Ça, c'est le code de connexion Valheim : il sert à rejoindre la partie, pas à récupérer les mods. Demandez à votre admin son code d'invitation ValhSync, ou l'adresse du serveur."
        }
        Key::Checking2 => "Interrogation du serveur…",
        Key::ConfirmKeyTitle => "Vérifiez l'empreinte",
        Key::ConfirmKeyBody => {
            "Vous ajoutez ce serveur par son adresse, sans code d'invitation. Comparez cette empreinte avec celle que l'administrateur vous a donnée : c'est elle qui garantit que les mods viennent bien de lui."
        }
        Key::Trust => "L'empreinte correspond, ajouter",
        Key::Added => "Serveur ajouté",
        Key::Launching => "Valheim démarre via Steam…",
        Key::SyncDone => "Synchronisation terminée",
        Key::RolledBack => "Version précédente restaurée",
        Key::Downloading => "Téléchargement",
        Key::Applying => "Application des changements…",
        Key::Contacting => "Connexion au serveur…",
        Key::PlanInstall | Key::ModToInstall => "à installer",
        Key::PlanUpdate | Key::ModToUpdate => "à mettre à jour",
        Key::PlanRemove => "à retirer",
        Key::PlanQuarantine => "à mettre en quarantaine",
        Key::PlanDownload => "à télécharger",
        Key::ConfirmTitle => "Ce que ValhSync va faire",
        Key::ConfirmBody => {
            "Première synchronisation avec ce serveur. Les fichiers inconnus trouvés dans les dossiers de mods seront déplacés en quarantaine, pas supprimés. Une sauvegarde est faite avant tout changement."
        }
        Key::Forget => "Oublier ce serveur",
        Key::Refresh => "Revérifier",
        Key::Settings => "Réglages",
        Key::Language => "Langue",
        Key::Busy => "Opération en cours…",
        Key::KeyFingerprint => "Empreinte de la clé",
        Key::TrustLine => {
            "Fichiers signés par ce serveur et vérifiés un par un. ValhSync n'écrit que dans le dossier du jeu (BepInEx) et n'exécute rien lui-même : c'est Steam qui lance Valheim."
        }
        Key::Installed => "installés",
        Key::Nothing => "Rien à faire.",
        Key::ServerMods => "Mods du serveur",
        Key::ShowAll => "Tout afficher",
        Key::ModInstalled => "installé",
        Key::Speed => "vitesse",
        Key::Remaining => "restant",
    }
}

fn en(key: Key) -> &'static str {
    match key {
        Key::Subtitle => "Your mods, matched to the server. Then play.",
        Key::AddServer => "Add a server",
        Key::NoServerHint => {
            "Paste the invite code your admin sent you, or drop their valhsync-invite.txt next to valhsync.exe."
        }
        Key::Checking => "Checking with the server…",
        Key::GameUp => "Game server online",
        Key::GameDown => "Game server stopped — you will not be able to join",
        Key::UpToDate => "Up to date",
        Key::FirstSync => "First sync",
        Key::Pending => "Update available",
        Key::Play => "PLAY",
        Key::Update => "UPDATE",
        Key::Launching2 => "Starting Valheim",
        Key::Repair => "Repair",
        Key::RepairHint => {
            "Check every file again and put back anything that differs from the server, your configuration included."
        }
        Key::ForgetServer => "Remove this server from the list",
        Key::SetAsideCount => "mods set aside",
        Key::Rollback => "Go back to the previous version",
        Key::SetAside => "Mods set aside",
        Key::SetAsideHint => {
            "Mod files found on your machine but absent from the server's pack: ValhSync moved them so they cannot block your connection. Nothing is deleted, you can take them back."
        }
        Key::SetAsideNone => "Nothing set aside.",
        Key::GameFolder => "Game folder",
        Key::GameFolderHint => {
            "Leave empty and ValhSync finds Valheim by itself through Steam. Fill it in only if detection fails."
        }
        Key::GameFolderInUse => "Currently used",
        Key::GameFolderAuto => "Automatic detection",
        Key::ThisServer => "This server",
        Key::AboutValhSync => "ValhSync",
        Key::OpenFolder => "Open the folder",
        Key::ResetAll => "Reset everything",
        Key::ResetAllHint => {
            "Forgets every server and every ValhSync setting. Leaves the game and the installed mods alone."
        }
        Key::ResetDone => "ValhSync reset.",
        Key::Apply => "Apply",
        Key::Cancel => "Cancel",
        Key::Close => "Close",
        Key::InvitePrompt => {
            "Invite code, or simply the game server's address — the same one you use in Valheim"
        }
        Key::InviteHint => "valhsync1:…  or  myserver.example.org:2456",
        Key::JoinCodeNotAnAddress => {
            "That is Valheim's join code: it gets you into the game, not to the mods. Ask your admin for their ValhSync invite code, or the server's address."
        }
        Key::Checking2 => "Asking the server…",
        Key::ConfirmKeyTitle => "Check the fingerprint",
        Key::ConfirmKeyBody => {
            "You are adding this server by address, without an invite code. Compare this fingerprint with the one your admin gave you: it is what proves the mods really come from them."
        }
        Key::Trust => "The fingerprint matches, add it",
        Key::Added => "Server added",
        Key::Launching => "Valheim is starting via Steam…",
        Key::SyncDone => "Sync complete",
        Key::RolledBack => "Previous version restored",
        Key::Downloading => "Downloading",
        Key::Applying => "Applying changes…",
        Key::Contacting => "Contacting the server…",
        Key::PlanInstall | Key::ModToInstall => "to install",
        Key::PlanUpdate | Key::ModToUpdate => "to update",
        Key::PlanRemove => "to remove",
        Key::PlanQuarantine => "to quarantine",
        Key::PlanDownload => "to download",
        Key::ConfirmTitle => "What ValhSync is about to do",
        Key::ConfirmBody => {
            "First sync with this server. Unknown files found in the mod folders are moved to a quarantine, never deleted. A backup is taken before anything changes."
        }
        Key::Forget => "Forget this server",
        Key::Refresh => "Check again",
        Key::Settings => "Settings",
        Key::Language => "Language",
        Key::Busy => "Working…",
        Key::KeyFingerprint => "Key fingerprint",
        Key::TrustLine => {
            "Files are signed by this server and verified one by one. ValhSync only writes inside the game folder (BepInEx) and runs nothing itself: Steam starts Valheim."
        }
        Key::Installed | Key::ModInstalled => "installed",
        Key::Nothing => "Nothing to do.",
        Key::ServerMods => "Server mods",
        Key::ShowAll => "Show all",
        Key::Speed => "speed",
        Key::Remaining => "left",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_unless_french_was_chosen() {
        assert_eq!(Lang::detect(Some("fr-FR")), Lang::Fr);
        assert_eq!(Lang::detect(Some("en")), Lang::En);
        assert_eq!(Lang::detect(Some("de")), Lang::En);
        assert_eq!(Lang::detect(None), Lang::En);
        assert_eq!(Lang::detect(None), Lang::En);
    }
}
