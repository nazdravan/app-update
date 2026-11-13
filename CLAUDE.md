# CLAUDE.md

`app-update` este o bibliotecă Rust care actualizează aplicațiile noastre desktop egui
din GitHub Releases (primul utilizator: Furnica). Specificația e în
`~/Projects/school/furnica/docs/specs/2026-11-13-app-update-design.md`.

## Comenzi

- `cargo test`
- `cargo clippy --all-targets -- -D warnings`

## Reguli

- Windows 7: fiecare dependență trebuie să se compileze pentru `i686-win7-windows-gnu`;
  TLS doar cu rustls și rădăcini webpki incluse, niciodată TLS-ul sistemului.
- Codul și comentariile sunt în engleză; textele dialogului (RO/EN/RU) stau în `texts.rs`.
- Fără atribuire în commit-uri: nici trailer, nici `Co-Authored-By`, nici link de sesiune.
