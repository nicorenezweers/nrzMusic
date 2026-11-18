#!/bin/sh
# Android-App (android/) bauen und als static/nrzMusic.apk ablegen – der Server bettet sie ein
# und bietet sie unter /static/nrzMusic.apk zum Herunterladen an (Anmeldeseite, Konto).
# Braucht: JDK 17 und das Android-SDK (Plattform 35, Build-Tools); Signierschlüssel über
# NRZMUSIC_KEYSTORE / NRZMUSIC_KEYSTORE_PASSWORD in ~/.gradle/gradle.properties (nie im Repo).
set -eu
cd "$(dirname "$0")/../android"

if [ -z "${JAVA_HOME:-}" ] && [ -d /opt/homebrew/opt/openjdk@17 ]; then
  JAVA_HOME=/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home
  export JAVA_HOME
fi
if [ ! -f local.properties ] && [ -d /opt/homebrew/share/android-commandlinetools ]; then
  echo "sdk.dir=/opt/homebrew/share/android-commandlinetools" > local.properties
fi

./gradlew --console=plain -q assembleRelease
apk=app/build/outputs/apk/release/app-release.apk
if [ ! -f "$apk" ]; then
  echo "Keine signierte APK – fehlt NRZMUSIC_KEYSTORE?" >&2
  exit 1
fi
cp "$apk" ../static/nrzMusic.apk
echo "Android-App gebaut: static/nrzMusic.apk ($(du -h ../static/nrzMusic.apk | cut -f1))"
