// メニューバーの右端に言語切替リンクを足す。
//
// mdbook本体に言語切替UIは無く、テーマ(index.hbs)を丸ごと抱えると本体の
// バージョンアップに追随する手間が出るため、JSで差し込む方式にしている
// (docs/decisions 0064)。
//
// URLは `/` が日本語、`/en/` が英語(0064 決定4)。現在のパスに `/en/` が
// 含まれるかどうかで、相手の言語の同じページへのリンクを作る。
(function () {
  "use strict";

  // path_to_root は mdbook が各ページに埋め込む「このページからサイト
  // ルートまでの相対パス」(例: "../")。英語版では book/en/ がルートになる。
  var pathToRoot = document.querySelector("html").dataset.pathToRoot || "";

  var isEnglish = /(^|\/)en\//.test(window.location.pathname);
  var label = isEnglish ? "日本語" : "English";
  // 英語版から日本語版へはルートの1つ上、日本語版から英語版へはルート配下の en/。
  var href = isEnglish ? pathToRoot + "../" : pathToRoot + "en/";

  var rightButtons = document.querySelector(".right-buttons");
  if (!rightButtons) {
    return;
  }

  var link = document.createElement("a");
  link.href = href;
  link.title = isEnglish ? "Read in Japanese" : "Read in English";
  link.className = "lang-switch";
  link.textContent = label;
  rightButtons.appendChild(link);
})();
