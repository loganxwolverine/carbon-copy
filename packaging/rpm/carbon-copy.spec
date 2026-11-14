# Paquet binaire : le programme est compilé par packaging/build.sh puis
# copié tel quel (variable « stage »). Les dépendances vers GTK et
# libadwaita sont calculées automatiquement par rpmbuild.
Name:           carbon-copy
Version:        %{ver}
Release:        %{rel}
Summary:        Copie conforme de vos DVD et Blu-ray en MKV (interface GTK4 pour MakeMKV)
License:        GPL-3.0-or-later
URL:            https://github.com/loganxwolverine/carbon-copy
Requires:       flatpak
Requires:       mkvtoolnix
Requires:       hicolor-icon-theme

%description
Carbon Copy pilote MakeMKV pour extraire vos DVD et Blu-ray en fichiers MKV :
reconnaissance du film avec son affiche, choix automatique des pistes, nommage
complet du fichier et éjection en fin de copie. MakeMKV est installé et mis à
jour automatiquement depuis Flathub.

%global debug_package %{nil}
%global __brp_strip %{nil}

%install
cp -a %{stage}/. %{buildroot}/

%post
modprobe sg >/dev/null 2>&1 || :
udevadm control --reload-rules >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=scsi_generic >/dev/null 2>&1 || :

%files
%license /usr/share/licenses/carbon-copy/LICENSE
/usr/bin/carbon-copy
/usr/share/applications/io.github.carboncopy.CarbonCopy.desktop
/usr/share/metainfo/io.github.carboncopy.CarbonCopy.metainfo.xml
/usr/share/icons/hicolor/scalable/apps/io.github.carboncopy.CarbonCopy.svg
/usr/lib/udev/rules.d/70-carbon-copy.rules
/usr/lib/modules-load.d/carbon-copy.conf
%dir /usr/share/licenses/carbon-copy
