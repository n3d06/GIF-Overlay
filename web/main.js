'use strict';

const REPO = 'n3d06/GIF-Overlay';
const size = (b) => (b < 1048576 ? `${Math.round(b / 1024)} KB` : `${(b / 1048576).toFixed(1)} MB`);

// Point the button at the latest installer and show the version (the static link still works without this)
fetch(`https://api.github.com/repos/${REPO}/releases/latest`)
  .then((r) => (r.ok ? r.json() : Promise.reject()))
  .then((rel) => {
    const files = rel.assets || [];
    const setup = files.find((a) => /setup.*\.exe$/i.test(a.name));
    const zip = files.find((a) => /portable.*\.zip$/i.test(a.name));
    if (setup) document.getElementById('download').href = setup.browser_download_url;
    const parts = [rel.tag_name, setup && `installer ${size(setup.size)}`, zip && `portable ${size(zip.size)}`];
    document.getElementById('release').textContent =
      `${parts.filter(Boolean).join(' · ')} · Windows 10 / 11 · MIT licensed`;
  })
  .catch(() => {});

const status = document.getElementById('copy-status');
document.getElementById('copy').addEventListener('click', async (e) => {
  const button = e.currentTarget;
  const code = document.getElementById('winget');
  try {
    await navigator.clipboard.writeText(code.textContent);
    button.textContent = 'Copied';
    status.textContent = 'Copied';
  } catch {
    // clipboard blocked: select the command so Ctrl+C works
    getSelection().selectAllChildren(code);
    button.textContent = 'Press Ctrl+C';
    status.textContent = 'Copy failed. The command is selected; press Control C to copy it.';
  }
  setTimeout(() => {
    button.textContent = 'Copy';
    status.textContent = '';
  }, 1800);
});
