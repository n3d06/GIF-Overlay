//! Interface languages. Every text in the code is its English wording; `t` swaps it for the chosen
//! language by looking it up in `ROWS` (a text with no row, such as an OS error, stays as it is).

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::lang_extra::EXTRA;

/// Column of `ROWS` (then of `EXTRA`) for each language, also the order of the View > Language menu.
pub const CODES: [&str; 18] = ["en", "vi", "zh", "ja", "ko", "es", "fr", "de", "pt", "ru", "zht", "id", "tr", "it", "pl", "uk", "th", "hi"];
pub const NAMES: [&str; 18] = ["English", "Tiếng Việt", "简体中文", "日本語", "한국어", "Español", "Français", "Deutsch", "Português", "Русский", "繁體中文", "Bahasa Indonesia", "Türkçe", "Italiano", "Polski", "Українська", "ไทย", "हिन्दी"];

static CUR: AtomicUsize = AtomicUsize::new(0);

pub fn set(i: usize) {
    CUR.store(i.min(CODES.len() - 1), Ordering::Relaxed);
}

/// Language for a Windows LANGID (the system's display language): the primary language id picks the column.
pub fn detect(langid: u16) -> usize {
    if matches!(langid, 0x0404 | 0x0C04 | 0x1404 | 0x7C04) {
        return 10; // Traditional Chinese: Taiwan, Hong Kong, Macao, zh-CHT
    }
    match langid & 0x3FF {
        0x2A => 1,
        0x04 => 2, // the other Chinese variants read the Simplified texts
        0x11 => 3,
        0x12 => 4,
        0x0A => 5,
        0x0C => 6,
        0x07 => 7,
        0x16 => 8,
        0x19 => 9,
        0x21 => 11,
        0x1F => 12,
        0x10 => 13,
        0x15 => 14,
        0x22 => 15,
        0x1E => 16,
        0x39 => 17,
        _ => 0,
    }
}

pub fn t(s: &str) -> String {
    let i = CUR.load(Ordering::Relaxed);
    if i == 0 {
        return s.to_string();
    }
    let Some(n) = ROWS.iter().position(|r| r[0] == s) else { return s.to_string() };
    let text = if i < 10 { ROWS[n][i] } else { EXTRA[n][i - 9] }; // EXTRA starts with the English key
    text.to_string()
}

/// `t` for a text with `{}` placeholders, filled in order with `args`.
pub fn tf(s: &str, args: &[&str]) -> String {
    let tpl = t(s);
    let mut out = String::new();
    let mut args = args.iter();
    for (i, part) in tpl.split("{}").enumerate() {
        if i > 0 {
            out += args.next().copied().unwrap_or("");
        }
        out += part;
    }
    out
}

// en, vi, zh, ja, ko, es, fr, de, pt, ru
const ROWS: &[[&str; 10]] = &[
    // right-click menu of an image
    ["Rotate Left", "Xoay trái", "向左旋转", "左に回転", "왼쪽으로 회전", "Girar a la izquierda", "Pivoter à gauche", "Nach links drehen", "Girar para a esquerda", "Повернуть влево"],
    ["Rotate Right", "Xoay phải", "向右旋转", "右に回転", "오른쪽으로 회전", "Girar a la derecha", "Pivoter à droite", "Nach rechts drehen", "Girar para a direita", "Повернуть вправо"],
    ["Flip Horizontal", "Lật ngang", "水平翻转", "左右反転", "좌우 반전", "Voltear horizontalmente", "Retourner horizontalement", "Horizontal spiegeln", "Inverter horizontalmente", "Отразить по горизонтали"],
    ["Flip Vertical", "Lật dọc", "垂直翻转", "上下反転", "상하 반전", "Voltear verticalmente", "Retourner verticalement", "Vertikal spiegeln", "Inverter verticalmente", "Отразить по вертикали"],
    ["Scale", "Tỉ lệ", "缩放", "拡大率", "크기", "Escala", "Échelle", "Skalierung", "Escala", "Масштаб"],
    ["Opacity", "Độ mờ", "不透明度", "不透明度", "불투명도", "Opacidad", "Opacité", "Deckkraft", "Opacidade", "Непрозрачность"],
    ["Transform", "Biến đổi", "变换", "変形", "변형", "Transformar", "Transformer", "Transformieren", "Transformar", "Преобразование"],
    ["Reset to Default", "Đặt lại mặc định", "恢复默认", "初期設定に戻す", "기본값으로 재설정", "Restablecer valores", "Réinitialiser", "Zurücksetzen", "Restaurar padrão", "Сбросить"],
    ["Lock Window (Ctrl+click to unlock)", "Khóa cửa sổ (Ctrl+nhấp để mở khóa)", "锁定窗口（Ctrl+单击解锁）", "ウィンドウをロック（Ctrl+クリックで解除）", "창 잠금 (Ctrl+클릭으로 해제)", "Bloquear ventana (Ctrl+clic para desbloquear)", "Verrouiller la fenêtre (Ctrl+clic pour déverrouiller)", "Fenster sperren (Strg+Klick zum Entsperren)", "Bloquear janela (Ctrl+clique para desbloquear)", "Заблокировать окно (Ctrl+щелчок — разблокировать)"],
    ["Dashboard", "Bảng điều khiển", "主面板", "ダッシュボード", "대시보드", "Panel", "Tableau de bord", "Übersicht", "Painel", "Панель"],
    ["Replace this image...", "Thay ảnh này...", "替换此图片...", "この画像を置き換え...", "이 이미지 바꾸기...", "Reemplazar esta imagen...", "Remplacer cette image...", "Dieses Bild ersetzen...", "Substituir esta imagem...", "Заменить это изображение..."],
    ["Paste image or link\tCtrl+V", "Dán ảnh hoặc liên kết\tCtrl+V", "粘贴图片或链接\tCtrl+V", "画像またはリンクを貼り付け\tCtrl+V", "이미지 또는 링크 붙여넣기\tCtrl+V", "Pegar imagen o enlace\tCtrl+V", "Coller une image ou un lien\tCtrl+V", "Bild oder Link einfügen\tStrg+V", "Colar imagem ou link\tCtrl+V", "Вставить изображение или ссылку\tCtrl+V"],
    ["Copy image\tCtrl+C", "Sao chép ảnh\tCtrl+C", "复制图片\tCtrl+C", "画像をコピー\tCtrl+C", "이미지 복사\tCtrl+C", "Copiar imagen\tCtrl+C", "Copier l'image\tCtrl+C", "Bild kopieren\tStrg+C", "Copiar imagem\tCtrl+C", "Копировать изображение\tCtrl+C"],
    ["Show in folder", "Mở thư mục chứa ảnh", "在文件夹中显示", "フォルダーで表示", "폴더에서 보기", "Mostrar en carpeta", "Afficher dans le dossier", "Im Ordner anzeigen", "Mostrar na pasta", "Показать в папке"],
    ["Settings", "Cài đặt", "设置", "設定", "설정", "Ajustes", "Paramètres", "Einstellungen", "Configurações", "Настройки"],
    ["Always on Top", "Luôn ở trên cùng", "始终置顶", "常に手前に表示", "항상 위에", "Siempre visible", "Toujours au premier plan", "Immer im Vordergrund", "Sempre no topo", "Поверх всех окон"],
    ["Pause / Play\tSpace", "Tạm dừng / Phát\tSpace", "暂停 / 播放\tSpace", "一時停止 / 再生\tSpace", "일시정지 / 재생\tSpace", "Pausa / Reproducir\tSpace", "Pause / Lecture\tSpace", "Pause / Wiedergabe\tSpace", "Pausar / Reproduzir\tSpace", "Пауза / Воспроизведение\tSpace"],
    ["Speed\t- / +", "Tốc độ\t- / +", "速度\t- / +", "速度\t- / +", "속도\t- / +", "Velocidad\t- / +", "Vitesse\t- / +", "Geschwindigkeit\t- / +", "Velocidade\t- / +", "Скорость\t- / +"],
    ["Close window\tEsc", "Đóng cửa sổ\tEsc", "关闭窗口\tEsc", "ウィンドウを閉じる\tEsc", "창 닫기\tEsc", "Cerrar ventana\tEsc", "Fermer la fenêtre\tEsc", "Fenster schließen\tEsc", "Fechar janela\tEsc", "Закрыть окно\tEsc"],
    ["Quit GIF Overlay", "Thoát GIF Overlay", "退出 GIF Overlay", "GIF Overlay を終了", "GIF Overlay 종료", "Salir de GIF Overlay", "Quitter GIF Overlay", "GIF Overlay beenden", "Sair do GIF Overlay", "Выйти из GIF Overlay"],
    // file dialog, window titles
    ["Images", "Hình ảnh", "图片", "画像", "이미지", "Imágenes", "Images", "Bilder", "Imagens", "Изображения"],
    ["GIF Files", "Tệp GIF", "GIF 文件", "GIF ファイル", "GIF 파일", "Archivos GIF", "Fichiers GIF", "GIF-Dateien", "Arquivos GIF", "Файлы GIF"],
    ["All Files", "Tất cả tệp", "所有文件", "すべてのファイル", "모든 파일", "Todos los archivos", "Tous les fichiers", "Alle Dateien", "Todos os arquivos", "Все файлы"],
    ["Select Image", "Chọn ảnh", "选择图片", "画像を選択", "이미지 선택", "Seleccionar imagen", "Sélectionner une image", "Bild auswählen", "Selecionar imagem", "Выбор изображения"],
    ["Error", "Lỗi", "错误", "エラー", "오류", "Error", "Erreur", "Fehler", "Erro", "Ошибка"],
    ["GIF Overlay - loading...", "GIF Overlay - đang tải...", "GIF Overlay - 正在加载...", "GIF Overlay - 読み込み中...", "GIF Overlay - 불러오는 중...", "GIF Overlay - cargando...", "GIF Overlay - chargement...", "GIF Overlay - lädt...", "GIF Overlay - carregando...", "GIF Overlay - загрузка..."],
    ["Loading...", "Đang tải...", "正在加载...", "読み込み中...", "불러오는 중...", "Cargando...", "Chargement...", "Lädt...", "Carregando...", "Загрузка..."],
    ["GIF Overlay - downloading...", "GIF Overlay - đang tải xuống...", "GIF Overlay - 正在下载...", "GIF Overlay - ダウンロード中...", "GIF Overlay - 다운로드 중...", "GIF Overlay - descargando...", "GIF Overlay - téléchargement...", "GIF Overlay - Download läuft...", "GIF Overlay - baixando...", "GIF Overlay - скачивание..."],
    // messages
    ["Failed to load image: {}", "Không tải được ảnh: {}", "无法加载图片：{}", "画像を読み込めませんでした: {}", "이미지를 불러오지 못했습니다: {}", "No se pudo cargar la imagen: {}", "Impossible de charger l'image : {}", "Bild konnte nicht geladen werden: {}", "Falha ao carregar a imagem: {}", "Не удалось загрузить изображение: {}"],
    ["the decoder crashed on this file", "bộ giải mã bị lỗi với tệp này", "解码器在处理此文件时崩溃", "このファイルでデコーダーがクラッシュしました", "이 파일에서 디코더가 중단되었습니다", "el decodificador falló con este archivo", "le décodeur a planté sur ce fichier", "der Decoder ist bei dieser Datei abgestürzt", "o decodificador falhou com este arquivo", "декодер аварийно завершился на этом файле"],
    ["Cannot save the pasted image next to the app.", "Không lưu được ảnh đã dán cạnh ứng dụng.", "无法将粘贴的图片保存到程序旁边。", "貼り付けた画像をアプリの隣に保存できません。", "붙여넣은 이미지를 앱 옆에 저장할 수 없습니다.", "No se puede guardar la imagen pegada junto a la aplicación.", "Impossible d'enregistrer l'image collée à côté de l'application.", "Das eingefügte Bild kann nicht neben der App gespeichert werden.", "Não foi possível salvar a imagem colada ao lado do aplicativo.", "Не удалось сохранить вставленное изображение рядом с приложением."],
    ["The clipboard has no image, image file or image link.", "Bộ nhớ tạm không có ảnh, tệp ảnh hay liên kết ảnh.", "剪贴板中没有图片、图片文件或图片链接。", "クリップボードに画像、画像ファイル、画像リンクがありません。", "클립보드에 이미지, 이미지 파일 또는 이미지 링크가 없습니다.", "El portapapeles no contiene imagen, archivo de imagen ni enlace de imagen.", "Le presse-papiers ne contient ni image, ni fichier image, ni lien d'image.", "Die Zwischenablage enthält kein Bild, keine Bilddatei und keinen Bildlink.", "A área de transferência não tem imagem, arquivo de imagem nem link de imagem.", "В буфере обмена нет изображения, файла изображения или ссылки на изображение."],
    ["Cannot create the pasted folder next to the app.", "Không tạo được thư mục pasted cạnh ứng dụng.", "无法在程序旁边创建 pasted 文件夹。", "アプリの隣に pasted フォルダーを作成できません。", "앱 옆에 pasted 폴더를 만들 수 없습니다.", "No se puede crear la carpeta pasted junto a la aplicación.", "Impossible de créer le dossier pasted à côté de l'application.", "Der Ordner pasted kann nicht neben der App angelegt werden.", "Não foi possível criar a pasta pasted ao lado do aplicativo.", "Не удалось создать папку pasted рядом с приложением."],
    ["Download failed: {}", "Tải xuống thất bại: {}", "下载失败：{}", "ダウンロードに失敗しました: {}", "다운로드 실패: {}", "Error de descarga: {}", "Échec du téléchargement : {}", "Download fehlgeschlagen: {}", "Falha no download: {}", "Не удалось скачать: {}"],
    // updates
    ["Check for updates", "Kiểm tra cập nhật", "检查更新", "更新を確認", "업데이트 확인", "Buscar actualizaciones", "Rechercher des mises à jour", "Nach Updates suchen", "Verificar atualizações", "Проверить обновления"],
    ["Version {} is available (you have {}).\nOpen the download page?", "Đã có phiên bản {} (bạn đang dùng {}).\nMở trang tải xuống?", "有新版本 {}（当前版本 {}）。\n打开下载页面？", "バージョン {} が利用可能です（現在 {}）。\nダウンロードページを開きますか？", "버전 {}을(를) 사용할 수 있습니다 (현재 {}).\n다운로드 페이지를 여시겠습니까?", "Hay una versión {} disponible (tienes la {}).\n¿Abrir la página de descarga?", "La version {} est disponible (vous avez la {}).\nOuvrir la page de téléchargement ?", "Version {} ist verfügbar (Sie haben {}).\nDownload-Seite öffnen?", "A versão {} está disponível (você tem a {}).\nAbrir a página de download?", "Доступна версия {} (у вас {}).\nОткрыть страницу загрузки?"],
    ["You are up to date (version {}).", "Bạn đang dùng bản mới nhất (phiên bản {}).", "已是最新版本（{}）。", "最新バージョンです（{}）。", "최신 버전입니다 ({}).", "Estás al día (versión {}).", "Vous êtes à jour (version {}).", "Sie sind auf dem neuesten Stand (Version {}).", "Você está atualizado (versão {}).", "У вас последняя версия ({})."],
    ["Could not check for updates: {}", "Không kiểm tra được cập nhật: {}", "无法检查更新：{}", "更新を確認できませんでした: {}", "업데이트를 확인할 수 없습니다: {}", "No se pudo buscar actualizaciones: {}", "Impossible de rechercher les mises à jour : {}", "Updates konnten nicht geprüft werden: {}", "Não foi possível verificar atualizações: {}", "Не удалось проверить обновления: {}"],
    ["no release found", "không tìm thấy bản phát hành", "未找到发布版本", "リリースが見つかりません", "릴리스를 찾을 수 없음", "no se encontró ninguna versión", "aucune version trouvée", "keine Version gefunden", "nenhuma versão encontrada", "релиз не найден"],
    // dashboard
    ["No history yet\r\nUse File > Add image or drop an image here.", "Chưa có lịch sử\r\nDùng Tệp > Thêm ảnh hoặc thả ảnh vào đây.", "还没有历史记录\r\n使用 文件 > 添加图片，或将图片拖到此处。", "履歴はまだありません\r\nファイル > 画像を追加 を使うか、ここに画像をドロップしてください。", "아직 기록이 없습니다\r\n파일 > 이미지 추가를 사용하거나 이미지를 여기로 끌어다 놓으세요.", "Aún no hay historial\r\nUsa Archivo > Añadir imagen o suelta una imagen aquí.", "Aucun historique pour l'instant\r\nUtilisez Fichier > Ajouter une image ou déposez une image ici.", "Noch kein Verlauf\r\nDatei > Bild hinzufügen verwenden oder ein Bild hierher ziehen.", "Ainda não há histórico\r\nUse Arquivo > Adicionar imagem ou solte uma imagem aqui.", "История пуста\r\nВыберите Файл > Добавить изображение или перетащите изображение сюда."],
    ["{} (open)", "{} (đang mở)", "{}（已打开）", "{}（表示中）", "{} (열림)", "{} (abierta)", "{} (ouverte)", "{} (geöffnet)", "{} (aberta)", "{} (открыто)"],
    ["Open", "Mở", "打开", "開く", "열기", "Abrir", "Ouvrir", "Öffnen", "Abrir", "Открыть"],
    ["Copy path", "Sao chép đường dẫn", "复制路径", "パスをコピー", "경로 복사", "Copiar ruta", "Copier le chemin", "Pfad kopieren", "Copiar caminho", "Копировать путь"],
    ["Remove from history", "Xóa khỏi lịch sử", "从历史记录中移除", "履歴から削除", "기록에서 제거", "Quitar del historial", "Supprimer de l'historique", "Aus Verlauf entfernen", "Remover do histórico", "Удалить из истории"],
    ["Add image", "Thêm ảnh", "添加图片", "画像を追加", "이미지 추가", "Añadir imagen", "Ajouter une image", "Bild hinzufügen", "Adicionar imagem", "Добавить изображение"],
    ["How to use GIF Overlay", "Cách dùng GIF Overlay", "GIF Overlay 使用说明", "GIF Overlay の使い方", "GIF Overlay 사용 방법", "Cómo usar GIF Overlay", "Utilisation de GIF Overlay", "So verwenden Sie GIF Overlay", "Como usar o GIF Overlay", "Как пользоваться GIF Overlay"],
    ["Clear the history list?\nThe image files themselves are not deleted.", "Xóa danh sách lịch sử?\nCác tệp ảnh sẽ không bị xóa.", "清空历史记录列表？\n图片文件本身不会被删除。", "履歴リストを消去しますか？\n画像ファイル自体は削除されません。", "기록 목록을 지울까요?\n이미지 파일은 삭제되지 않습니다.", "¿Borrar la lista del historial?\nLos archivos de imagen no se eliminan.", "Effacer la liste de l'historique ?\nLes fichiers image ne sont pas supprimés.", "Verlaufsliste leeren?\nDie Bilddateien selbst werden nicht gelöscht.", "Limpar a lista do histórico?\nOs arquivos de imagem não são excluídos.", "Очистить список истории?\nСами файлы изображений не удаляются."],
    ["Clear history", "Xóa lịch sử", "清空历史记录", "履歴を消去", "기록 지우기", "Borrar historial", "Effacer l'historique", "Verlauf leeren", "Limpar histórico", "Очистить историю"],
    ["No open images", "Không có ảnh nào đang mở", "没有已打开的图片", "開いている画像はありません", "열린 이미지 없음", "No hay imágenes abiertas", "Aucune image ouverte", "Keine Bilder geöffnet", "Nenhuma imagem aberta", "Нет открытых изображений"],
    ["Minimize all", "Thu nhỏ tất cả", "全部最小化", "すべて最小化", "모두 최소화", "Minimizar todo", "Tout réduire", "Alle minimieren", "Minimizar tudo", "Свернуть все"],
    ["Restore all", "Khôi phục tất cả", "全部还原", "すべて元に戻す", "모두 복원", "Restaurar todo", "Tout restaurer", "Alle wiederherstellen", "Restaurar tudo", "Восстановить все"],
    ["Close all windows", "Đóng tất cả cửa sổ", "关闭所有窗口", "すべてのウィンドウを閉じる", "모든 창 닫기", "Cerrar todas las ventanas", "Fermer toutes les fenêtres", "Alle Fenster schließen", "Fechar todas as janelas", "Закрыть все окна"],
    // dashboard menu bar
    ["File", "Tệp", "文件", "ファイル", "파일", "Archivo", "Fichier", "Datei", "Arquivo", "Файл"],
    ["View", "Xem", "查看", "表示", "보기", "Ver", "Affichage", "Ansicht", "Exibir", "Вид"],
    ["Window", "Cửa sổ", "窗口", "ウィンドウ", "창", "Ventana", "Fenêtre", "Fenster", "Janela", "Окно"],
    ["Help", "Trợ giúp", "帮助", "ヘルプ", "도움말", "Ayuda", "Aide", "Hilfe", "Ajuda", "Справка"],
    ["Add image\tCtrl+O", "Thêm ảnh\tCtrl+O", "添加图片\tCtrl+O", "画像を追加\tCtrl+O", "이미지 추가\tCtrl+O", "Añadir imagen\tCtrl+O", "Ajouter une image\tCtrl+O", "Bild hinzufügen\tStrg+O", "Adicionar imagem\tCtrl+O", "Добавить изображение\tCtrl+O"],
    ["Reopen windows on start", "Mở lại các cửa sổ khi khởi động", "启动时重新打开窗口", "起動時にウィンドウを再度開く", "시작할 때 창 다시 열기", "Reabrir ventanas al iniciar", "Rouvrir les fenêtres au démarrage", "Fenster beim Start wieder öffnen", "Reabrir janelas ao iniciar", "Открывать окна при запуске"],
    ["Sort by recently opened", "Sắp xếp theo mở gần đây", "按最近打开排序", "最近開いた順", "최근에 연 순서", "Ordenar por abiertas recientemente", "Trier par ouverture récente", "Nach zuletzt geöffnet sortieren", "Ordenar por abertas recentemente", "Сначала недавно открытые"],
    ["Sort by name (A\u{2013}Z)", "Sắp xếp theo tên (A–Z)", "按名称排序（A–Z）", "名前順（A–Z）", "이름순 (A–Z)", "Ordenar por nombre (A–Z)", "Trier par nom (A–Z)", "Nach Name sortieren (A–Z)", "Ordenar por nome (A–Z)", "По имени (А–Я)"],
    ["Sort by name (Z\u{2013}A)", "Sắp xếp theo tên (Z–A)", "按名称排序（Z–A）", "名前の逆順（Z–A）", "이름 역순 (Z–A)", "Ordenar por nombre (Z–A)", "Trier par nom (Z–A)", "Nach Name sortieren (Z–A)", "Ordenar por nome (Z–A)", "По имени (Я–А)"],
    ["Sort by size (largest first)", "Sắp xếp theo kích thước (lớn nhất trước)", "按大小排序（从大到小）", "サイズ順（大きい順）", "크기순 (큰 것부터)", "Ordenar por tamaño (mayor primero)", "Trier par taille (plus grand d'abord)", "Nach Größe sortieren (größte zuerst)", "Ordenar por tamanho (maior primeiro)", "По размеру (сначала большие)"],
    ["Sort by size (smallest first)", "Sắp xếp theo kích thước (nhỏ nhất trước)", "按大小排序（从小到大）", "サイズ順（小さい順）", "크기순 (작은 것부터)", "Ordenar por tamaño (menor primero)", "Trier par taille (plus petit d'abord)", "Nach Größe sortieren (kleinste zuerst)", "Ordenar por tamanho (menor primeiro)", "По размеру (сначала маленькие)"],
    ["How to use", "Cách dùng", "使用说明", "使い方", "사용 방법", "Cómo usar", "Mode d'emploi", "Anleitung", "Como usar", "Как пользоваться"],
    ["Set as default viewer", "Đặt làm trình xem mặc định", "设为默认查看器", "既定のビューアーに設定", "기본 뷰어로 설정", "Establecer como visor predeterminado", "Définir comme visionneuse par défaut", "Als Standardbetrachter festlegen", "Definir como visualizador padrão", "Сделать программой просмотра по умолчанию"],
    ["Report a bug", "Báo lỗi", "报告问题", "不具合を報告", "버그 신고", "Informar de un error", "Signaler un bug", "Fehler melden", "Relatar um bug", "Сообщить об ошибке"],
    ["Version {}", "Phiên bản {}", "版本 {}", "バージョン {}", "버전 {}", "Versión {}", "Version {}", "Version {}", "Versão {}", "Версия {}"],
    ["Language", "Ngôn ngữ", "语言", "言語", "언어", "Idioma", "Langue", "Sprache", "Idioma", "Язык"],
    ["System default", "Theo hệ thống", "跟随系统", "システムの設定に従う", "시스템 기본값", "Predeterminado del sistema", "Par défaut du système", "Systemstandard", "Padrão do sistema", "Как в системе"],
    // "How to use" text, one line per row
    ["OPEN AN IMAGE", "MỞ ẢNH", "打开图片", "画像を開く", "이미지 열기", "ABRIR UNA IMAGEN", "OUVRIR UNE IMAGE", "BILD ÖFFNEN", "ABRIR UMA IMAGEM", "ОТКРЫТЬ ИЗОБРАЖЕНИЕ"],
    ["\u{2022} Use File > Add image, drop a file here, or double-click a thumbnail.", "\u{2022} Dùng Tệp > Thêm ảnh, thả tệp vào đây hoặc nhấp đúp vào hình thu nhỏ.", "\u{2022} 使用 文件 > 添加图片，将文件拖到此处，或双击缩略图。", "\u{2022} ファイル > 画像を追加、ここにファイルをドロップ、またはサムネイルをダブルクリックします。", "\u{2022} 파일 > 이미지 추가를 사용하거나 파일을 여기로 끌어다 놓거나 썸네일을 두 번 클릭하세요.", "\u{2022} Usa Archivo > Añadir imagen, suelta un archivo aquí o haz doble clic en una miniatura.", "\u{2022} Utilisez Fichier > Ajouter une image, déposez un fichier ici ou double-cliquez sur une miniature.", "\u{2022} Datei > Bild hinzufügen verwenden, eine Datei hierher ziehen oder eine Miniatur doppelklicken.", "\u{2022} Use Arquivo > Adicionar imagem, solte um arquivo aqui ou clique duas vezes em uma miniatura.", "\u{2022} Выберите Файл > Добавить изображение, перетащите файл сюда или дважды щёлкните миниатюру."],
    ["\u{2022} Ctrl+V pastes a screenshot, a copied file, or an image link (http / https).", "\u{2022} Ctrl+V dán ảnh chụp màn hình, tệp đã sao chép hoặc liên kết ảnh (http / https).", "\u{2022} Ctrl+V 可粘贴截图、复制的文件或图片链接（http / https）。", "\u{2022} Ctrl+V でスクリーンショット、コピーしたファイル、画像リンク（http / https）を貼り付けられます。", "\u{2022} Ctrl+V로 스크린샷, 복사한 파일 또는 이미지 링크(http / https)를 붙여넣을 수 있습니다.", "\u{2022} Ctrl+V pega una captura de pantalla, un archivo copiado o un enlace de imagen (http / https).", "\u{2022} Ctrl+V colle une capture d'écran, un fichier copié ou un lien d'image (http / https).", "\u{2022} Strg+V fügt einen Screenshot, eine kopierte Datei oder einen Bildlink (http / https) ein.", "\u{2022} Ctrl+V cola uma captura de tela, um arquivo copiado ou um link de imagem (http / https).", "\u{2022} Ctrl+V вставляет снимок экрана, скопированный файл или ссылку на изображение (http / https)."],
    ["\u{2022} Right-click a thumbnail: Open, Show in folder, Copy path (Ctrl+C), Remove (Delete).", "\u{2022} Nhấp chuột phải vào hình thu nhỏ: Mở, Mở thư mục chứa ảnh, Sao chép đường dẫn (Ctrl+C), Xóa (Delete).", "\u{2022} 右键单击缩略图：打开、在文件夹中显示、复制路径（Ctrl+C）、移除（Delete）。", "\u{2022} サムネイルを右クリック：開く、フォルダーで表示、パスをコピー（Ctrl+C）、削除（Delete）。", "\u{2022} 썸네일을 마우스 오른쪽 버튼으로 클릭: 열기, 폴더에서 보기, 경로 복사(Ctrl+C), 제거(Delete).", "\u{2022} Clic derecho en una miniatura: Abrir, Mostrar en carpeta, Copiar ruta (Ctrl+C), Quitar (Supr).", "\u{2022} Clic droit sur une miniature : Ouvrir, Afficher dans le dossier, Copier le chemin (Ctrl+C), Supprimer (Suppr).", "\u{2022} Rechtsklick auf eine Miniatur: Öffnen, Im Ordner anzeigen, Pfad kopieren (Strg+C), Entfernen (Entf).", "\u{2022} Clique com o botão direito em uma miniatura: Abrir, Mostrar na pasta, Copiar caminho (Ctrl+C), Remover (Delete).", "\u{2022} Правый щелчок по миниатюре: Открыть, Показать в папке, Копировать путь (Ctrl+C), Удалить (Delete)."],
    ["\u{2022} Right-click an empty spot for Add image and Paste.", "\u{2022} Nhấp chuột phải vào chỗ trống để Thêm ảnh hoặc Dán.", "\u{2022} 右键单击空白处可添加图片或粘贴。", "\u{2022} 空白部分を右クリックすると、画像の追加と貼り付けができます。", "\u{2022} 빈 곳을 마우스 오른쪽 버튼으로 클릭하면 이미지 추가와 붙여넣기를 할 수 있습니다.", "\u{2022} Clic derecho en un espacio vacío para Añadir imagen y Pegar.", "\u{2022} Clic droit dans un espace vide pour Ajouter une image et Coller.", "\u{2022} Rechtsklick auf eine leere Stelle: Bild hinzufügen und Einfügen.", "\u{2022} Clique com o botão direito em um espaço vazio para Adicionar imagem e Colar.", "\u{2022} Правый щелчок по пустому месту — Добавить изображение и Вставить."],
    ["ON AN IMAGE", "TRÊN ẢNH", "图片窗口", "画像ウィンドウ", "이미지 창", "EN UNA IMAGEN", "SUR UNE IMAGE", "AUF EINEM BILD", "EM UMA IMAGEM", "ОКНО ИЗОБРАЖЕНИЯ"],
    ["\u{2022} Drag the image to move it. Hover it to show the buttons at the top right: minimize, maximize and close.", "\u{2022} Kéo ảnh để di chuyển. Rê chuột lên ảnh để hiện các nút ở góc trên bên phải: thu nhỏ, phóng to và đóng.", "\u{2022} 拖动图片可移动。鼠标悬停时右上角会显示按钮：最小化、最大化和关闭。", "\u{2022} 画像をドラッグして移動します。マウスを重ねると右上にボタン（最小化、最大化、閉じる）が表示されます。", "\u{2022} 이미지를 끌어 이동합니다. 마우스를 올리면 오른쪽 위에 최소화, 최대화, 닫기 버튼이 나타납니다.", "\u{2022} Arrastra la imagen para moverla. Pasa el ratón por encima para ver los botones de arriba a la derecha: minimizar, maximizar y cerrar.", "\u{2022} Faites glisser l'image pour la déplacer. Survolez-la pour afficher les boutons en haut à droite : réduire, agrandir et fermer.", "\u{2022} Das Bild ziehen, um es zu verschieben. Beim Darüberfahren erscheinen oben rechts die Schaltflächen Minimieren, Maximieren und Schließen.", "\u{2022} Arraste a imagem para movê-la. Passe o mouse para mostrar os botões no canto superior direito: minimizar, maximizar e fechar.", "\u{2022} Перетащите изображение, чтобы переместить его. При наведении справа вверху появляются кнопки: свернуть, развернуть и закрыть."],
    ["\u{2022} Drag the right edge, bottom edge or the corner to resize. Ctrl + mouse wheel zooms.", "\u{2022} Kéo cạnh phải, cạnh dưới hoặc góc để đổi kích thước. Ctrl + con lăn chuột để phóng to/thu nhỏ.", "\u{2022} 拖动右边缘、下边缘或角可调整大小。Ctrl + 鼠标滚轮可缩放。", "\u{2022} 右端、下端、または角をドラッグしてサイズ変更します。Ctrl + マウスホイールで拡大縮小します。", "\u{2022} 오른쪽 가장자리, 아래쪽 가장자리 또는 모서리를 끌어 크기를 조절합니다. Ctrl + 마우스 휠로 확대/축소합니다.", "\u{2022} Arrastra el borde derecho, el inferior o la esquina para cambiar el tamaño. Ctrl + rueda del ratón hace zoom.", "\u{2022} Faites glisser le bord droit, le bord inférieur ou le coin pour redimensionner. Ctrl + molette pour zoomer.", "\u{2022} Rechten Rand, unteren Rand oder die Ecke ziehen, um die Größe zu ändern. Strg + Mausrad zoomt.", "\u{2022} Arraste a borda direita, a borda inferior ou o canto para redimensionar. Ctrl + roda do mouse dá zoom.", "\u{2022} Потяните правый или нижний край либо угол, чтобы изменить размер. Ctrl + колесо мыши — масштаб."],
    ["\u{2022} Maximize stretches the image over the whole screen; click it again to restore.", "\u{2022} Phóng to sẽ kéo giãn ảnh ra toàn màn hình; nhấp lần nữa để khôi phục.", "\u{2022} 最大化会将图片拉伸到整个屏幕；再次单击可还原。", "\u{2022} 最大化すると画像が画面全体に引き伸ばされます。もう一度クリックすると元に戻ります。", "\u{2022} 최대화하면 이미지가 화면 전체로 늘어납니다. 다시 클릭하면 복원됩니다.", "\u{2022} Maximizar estira la imagen por toda la pantalla; haz clic otra vez para restaurar.", "\u{2022} Agrandir étire l'image sur tout l'écran ; cliquez de nouveau pour restaurer.", "\u{2022} Maximieren dehnt das Bild über den ganzen Bildschirm; erneut klicken stellt es wieder her.", "\u{2022} Maximizar estica a imagem por toda a tela; clique novamente para restaurar.", "\u{2022} Развернуть растягивает изображение на весь экран; повторный щелчок возвращает прежний вид."],
    ["\u{2022} Drop another file on the image to replace it.", "\u{2022} Thả tệp khác lên ảnh để thay thế.", "\u{2022} 将另一个文件拖到图片上可替换它。", "\u{2022} 別のファイルを画像にドロップすると置き換えられます。", "\u{2022} 다른 파일을 이미지 위에 놓으면 바뀝니다.", "\u{2022} Suelta otro archivo sobre la imagen para reemplazarla.", "\u{2022} Déposez un autre fichier sur l'image pour la remplacer.", "\u{2022} Eine andere Datei auf das Bild ziehen, um es zu ersetzen.", "\u{2022} Solte outro arquivo sobre a imagem para substituí-la.", "\u{2022} Перетащите другой файл на изображение, чтобы заменить его."],
    ["\u{2022} Right-click for the menu: scale, opacity, rotate / flip, pause, lock, always on top, copy image, show in folder, replace image, dashboard, close and quit.", "\u{2022} Nhấp chuột phải để mở menu: tỉ lệ, độ mờ, xoay / lật, tạm dừng, khóa, luôn ở trên cùng, sao chép ảnh, mở thư mục chứa ảnh, thay ảnh, bảng điều khiển, đóng và thoát.", "\u{2022} 右键单击打开菜单：缩放、不透明度、旋转 / 翻转、暂停、锁定、置顶、复制图片、在文件夹中显示、替换图片、主面板、关闭和退出。", "\u{2022} 右クリックでメニュー：拡大率、不透明度、回転 / 反転、一時停止、ロック、常に手前、画像をコピー、フォルダーで表示、画像を置き換え、ダッシュボード、閉じる、終了。", "\u{2022} 마우스 오른쪽 버튼 메뉴: 크기, 불투명도, 회전 / 반전, 일시정지, 잠금, 항상 위에, 이미지 복사, 폴더에서 보기, 이미지 바꾸기, 대시보드, 닫기, 종료.", "\u{2022} Clic derecho para el menú: escala, opacidad, girar / voltear, pausa, bloquear, siempre visible, copiar imagen, mostrar en carpeta, reemplazar imagen, panel, cerrar y salir.", "\u{2022} Clic droit pour le menu : échelle, opacité, pivoter / retourner, pause, verrouiller, toujours au premier plan, copier l'image, afficher dans le dossier, remplacer l'image, tableau de bord, fermer et quitter.", "\u{2022} Rechtsklick öffnet das Menü: Skalierung, Deckkraft, Drehen / Spiegeln, Pause, Sperren, Immer im Vordergrund, Bild kopieren, Im Ordner anzeigen, Bild ersetzen, Übersicht, Schließen und Beenden.", "\u{2022} Clique com o botão direito para o menu: escala, opacidade, girar / inverter, pausar, bloquear, sempre no topo, copiar imagem, mostrar na pasta, substituir imagem, painel, fechar e sair.", "\u{2022} Правый щелчок открывает меню: масштаб, прозрачность, поворот / отражение, пауза, блокировка, поверх всех окон, копировать изображение, показать в папке, заменить изображение, панель, закрыть и выйти."],
    ["\u{2022} Keys: Esc closes, Space pauses, Left / Right step one frame, - / + change the speed, Up / Down change the opacity, Ctrl+C copies the image.", "\u{2022} Phím: Esc để đóng, Space để tạm dừng, Trái / Phải chuyển từng khung hình, - / + đổi tốc độ, Lên / Xuống đổi độ mờ, Ctrl+C để sao chép ảnh.", "\u{2022} 按键：Esc 关闭，Space 暂停，左 / 右 逐帧步进，- / + 调整速度，上 / 下 调整不透明度，Ctrl+C 复制图片。", "\u{2022} キー：Esc で閉じる、Space で一時停止、左 / 右 で 1 フレーム移動、- / + で速度変更、上 / 下 で不透明度変更、Ctrl+C で画像をコピー。", "\u{2022} 키: Esc 닫기, Space 일시정지, 왼쪽 / 오른쪽 한 프레임 이동, - / + 속도 변경, 위 / 아래 불투명도 변경, Ctrl+C 이미지 복사.", "\u{2022} Teclas: Esc cierra, Espacio pausa, Izquierda / Derecha avanzan un fotograma, - / + cambian la velocidad, Arriba / Abajo cambian la opacidad, Ctrl+C copia la imagen.", "\u{2022} Touches : Échap ferme, Espace met en pause, Gauche / Droite avance d'une image, - / + change la vitesse, Haut / Bas change l'opacité, Ctrl+C copie l'image.", "\u{2022} Tasten: Esc schließt, Leertaste pausiert, Links / Rechts springt ein Einzelbild, - / + ändert die Geschwindigkeit, Hoch / Runter ändert die Deckkraft, Strg+C kopiert das Bild.", "\u{2022} Teclas: Esc fecha, Espaço pausa, Esquerda / Direita avançam um quadro, - / + mudam a velocidade, Cima / Baixo mudam a opacidade, Ctrl+C copia a imagem.", "\u{2022} Клавиши: Esc — закрыть, Пробел — пауза, Влево / Вправо — один кадр, - / + — скорость, Вверх / Вниз — прозрачность, Ctrl+C — копировать изображение."],
    ["LOCK WINDOW", "KHÓA CỬA SỔ", "锁定窗口", "ウィンドウのロック", "창 잠금", "BLOQUEAR VENTANA", "VERROUILLER LA FENÊTRE", "FENSTER SPERREN", "BLOQUEAR JANELA", "БЛОКИРОВКА ОКНА"],
    ["\u{2022} A locked image is click-through, so it never gets in the way.", "\u{2022} Ảnh bị khóa cho phép nhấp xuyên qua, nên không bao giờ vướng víu.", "\u{2022} 锁定的图片可被点击穿透，因此不会妨碍操作。", "\u{2022} ロックした画像はクリックが下のウィンドウに通るため、邪魔になりません。", "\u{2022} 잠긴 이미지는 클릭이 그대로 통과하므로 방해가 되지 않습니다.", "\u{2022} Una imagen bloqueada deja pasar los clics, así que nunca estorba.", "\u{2022} Une image verrouillée laisse passer les clics et ne gêne donc jamais.", "\u{2022} Ein gesperrtes Bild lässt Klicks durch und stört so nie.", "\u{2022} Uma imagem bloqueada deixa os cliques passarem, então nunca atrapalha.", "\u{2022} Заблокированное изображение пропускает щелчки и никогда не мешает."],
    ["\u{2022} To unlock it, hold Ctrl and click the image.", "\u{2022} Để mở khóa, giữ Ctrl và nhấp vào ảnh.", "\u{2022} 要解锁，请按住 Ctrl 并单击图片。", "\u{2022} ロックを解除するには、Ctrl を押しながら画像をクリックします。", "\u{2022} 잠금을 해제하려면 Ctrl을 누른 채 이미지를 클릭하세요.", "\u{2022} Para desbloquearla, mantén Ctrl y haz clic en la imagen.", "\u{2022} Pour la déverrouiller, maintenez Ctrl et cliquez sur l'image.", "\u{2022} Zum Entsperren Strg gedrückt halten und auf das Bild klicken.", "\u{2022} Para desbloquear, segure Ctrl e clique na imagem.", "\u{2022} Чтобы разблокировать, удерживайте Ctrl и щёлкните изображение."],
    ["FOUND A PROBLEM?", "GẶP SỰ CỐ?", "发现问题？", "問題を見つけたら", "문제가 있나요?", "¿ENCONTRASTE UN PROBLEMA?", "UN PROBLÈME ?", "PROBLEM GEFUNDEN?", "ENCONTROU UM PROBLEMA?", "НАШЛИ ПРОБЛЕМУ?"],
    ["\u{2022} Use Help > Report a bug to open the issue page on GitHub.", "\u{2022} Dùng Trợ giúp > Báo lỗi để mở trang báo lỗi trên GitHub.", "\u{2022} 使用 帮助 > 报告问题 打开 GitHub 上的问题页面。", "\u{2022} ヘルプ > 不具合を報告 で GitHub の issue ページを開きます。", "\u{2022} 도움말 > 버그 신고로 GitHub 이슈 페이지를 엽니다.", "\u{2022} Usa Ayuda > Informar de un error para abrir la página de incidencias en GitHub.", "\u{2022} Utilisez Aide > Signaler un bug pour ouvrir la page des tickets sur GitHub.", "\u{2022} Mit Hilfe > Fehler melden öffnen Sie die Issue-Seite auf GitHub.", "\u{2022} Use Ajuda > Relatar um bug para abrir a página de issues no GitHub.", "\u{2022} Откройте Справка > Сообщить об ошибке, чтобы перейти на страницу проблем на GitHub."],
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row is filled in for every language, keeps the same `{}` placeholders and tab shortcuts
    /// as the English text, and no English key appears twice.
    #[test]
    fn rows_are_complete_and_consistent() {
        for (n, r) in ROWS.iter().enumerate() {
            for (i, text) in r.iter().enumerate() {
                assert!(!text.is_empty(), "row {n} ({}) has no {}", r[0], CODES[i]);
                assert_eq!(text.matches("{}").count(), r[0].matches("{}").count(), "placeholders: {} / {}", r[0], CODES[i]);
                assert_eq!(text.contains('\t'), r[0].contains('\t'), "tab shortcut: {} / {}", r[0], CODES[i]);
                assert_eq!(text.contains('\n'), r[0].contains('\n'), "line break: {} / {}", r[0], CODES[i]);
            }
            assert!(ROWS[..n].iter().all(|o| o[0] != r[0]), "duplicate key {}", r[0]);
        }
    }

    /// `EXTRA` lines up with `ROWS` (same English key per row) and is held to the same rules.
    #[test]
    fn extra_rows_line_up() {
        assert_eq!(EXTRA.len(), ROWS.len());
        for (n, r) in EXTRA.iter().enumerate() {
            assert_eq!(r[0], ROWS[n][0], "row {n} is out of order");
            for (i, text) in r.iter().enumerate() {
                assert!(!text.is_empty(), "row {n} ({}) has no {}", r[0], CODES[i + 9]);
                assert_eq!(text.matches("{}").count(), r[0].matches("{}").count(), "placeholders: {} / {}", r[0], CODES[i + 9]);
                assert_eq!(text.contains('\t'), r[0].contains('\t'), "tab shortcut: {} / {}", r[0], CODES[i + 9]);
                assert_eq!(text.contains('\n'), r[0].contains('\n'), "line break: {} / {}", r[0], CODES[i + 9]);
            }
        }
    }

    #[test]
    fn lookup_and_fill() {
        set(1);
        assert_eq!(t("Open"), "Mở");
        assert_eq!(t("not in the table"), "not in the table"); // unknown texts pass through
        assert_eq!(tf("Version {}", &["1.2"]), "Phiên bản 1.2");
        set(0);
        assert_eq!(tf("Version {} / {}", &["a", "b"]), "Version a / b");
        assert_eq!(detect(0x042A), 1); // vi-VN
        assert_eq!(detect(0x0804), 2); // zh-CN
        assert_eq!(detect(0x0404), 10); // zh-TW
        assert_eq!(detect(0x0415), 14); // pl-PL
        assert_eq!(detect(0x0409), 0); // en-US
    }
}
