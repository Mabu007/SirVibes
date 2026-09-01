# Firebase

Project: **sirvibes-44204**

## What lives here

`firestore.rules` — the ownership rules for everything the desktop app writes.

## Deploying the rules

The rules are **not** applied by building the app. They have to be published to
the project once, and again whenever this file changes:

```bash
npm install -g firebase-tools
firebase login
firebase deploy --only firestore:rules --project sirvibes-44204
```

Or paste the file into **Firebase console → Firestore → Rules → Publish**.

Until they are published the database keeps whatever rules it already has. If
it is still in test mode, anyone with the (shippable) client config can read
everyone's chats — so publish these before the first customer install.

## What is in the cloud, and what is not

| In Firestore | On the machine |
|---|---|
| account identity | API keys and secrets |
| chat titles, status, timestamps | media, renders, source footage |
| conversation messages (last 200, trimmed) | workspace files and projects |
| | skills, tool output, raw logs |

## What is deliberately absent

No Admin SDK key, no service account, no server credential of any kind is in
this repository or in the shipped application. The client config in
`src/lib/firebase.ts` identifies the project and is meant to ship; on its own
it grants nothing, because every read and write is checked against the rules
above using the signed-in user's own token.
