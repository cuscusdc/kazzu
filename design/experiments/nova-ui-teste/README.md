# Prévia da nova UI do Kazzu

No Linux, na raiz do repositório:

```sh
./nova-ui-teste/preview.sh
```

O script usa o Slint Viewer 1.19.1 pré-compilado com Skia. A primeira execução baixa e verifica o binário oficial. As seguintes usam o cache e recarregam a interface ao salvar os arquivos. Cargo e o SDK Android são dispensáveis para a prévia.

A janela usa o tamanho fixo de 390 × 780 pixels lógicos, em formato vertical. As faixas e ações de serviço usam os exemplos do HTML. A reprodução simulada permite trocar de faixa, buscar no tempo e testar a fila e as configurações. A seção **Aparência → Vidro e textura** ajusta o desfoque nativo, a transparência, o brilho, a borda e o grão. O modo **Economia de bateria** desativa o desfoque e a textura.

Para verificar os arquivos ou capturar uma tela específica:

```sh
./nova-ui-teste/preview.sh --check
printf '{"preview":"player"}\n' > /tmp/kazzu-preview.json
./nova-ui-teste/preview.sh --load-data /tmp/kazzu-preview.json --screenshot /tmp/kazzu-player.png
```

A prévia é independente do APK. Os acabamentos são feitos com blur, sombras, camadas 2D e formas arredondadas. Refração e perspectiva CSS foram adaptadas com esses recursos nativos.

## Capas do catálogo de demonstração

As capas ficam em `assets/` para a prévia funcionar offline. As playlists de exemplo usam arte própria.

- [Pablo Honey · Radiohead](https://music.apple.com/us/album/pablo-honey/1097862062)
- [Bleach · Nirvana](https://music.apple.com/us/album/bleach/6830857209)
- [Meantime · Helmet](https://music.apple.com/us/album/meantime/1440915669)
- [In Utero · Nirvana](https://music.apple.com/us/album/in-utero-20th-anniversary-edition/1440858699)
- [Writer’s Block · Peter Bjorn and John](https://music.apple.com/us/album/young-folks/1442990312?i=1442990320)
- [Demon Days · Gorillaz](https://music.apple.com/us/album/demon-days/850571319)
- [Lateralus · TOOL](https://music.apple.com/us/album/lateralus/1474185412)
- [Lovelight · Alstroemeria Records](https://www.diverse.direct/alstroemeria-records/acvs0018/)
- [rearviewmirror · Pearl Jam](https://music.apple.com/us/album/rearviewmirror-greatest-hits-1991-2003/1826659396)
- [Circo · Venere Vai Venus](https://music.apple.com/us/album/circo-single/1697232565)
- [O Silêncio Que Precede o Esporro · O Rappa](https://music.apple.com/us/album/o-sil%C3%AAncio-que-precede-o-esporro/156745656)
