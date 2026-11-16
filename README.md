# RetopoEZ

Retopo nhanh cho Maya: vẽ nét lên mesh high-poly → chọn 2 hoặc 4 nét → tự sinh lưới quad bám bề mặt → xuất về Maya.
Mọi thao tác nằm trong một viewer độc lập (Rust + wgpu). Maya chỉ có 2 nút.

## Cài (Maya, mọi máy — không cần Rust)
Dán vào Script Editor (tab Python):
```python
import urllib.request, os, sys
d = os.path.join(os.path.expanduser('~'), 'RetopoEZ'); os.makedirs(d, exist_ok=True)
urllib.request.urlretrieve('https://raw.githubusercontent.com/hoathe01/RetopoEZ/main/retopoez.py', os.path.join(d, 'retopoez.py'))
sys.path.insert(0, d); import retopoez; retopoez.show()
```
Lần đầu bấm **Open viewer** sẽ tự tải viewer (vài MB) từ GitHub Releases; những lần sau dùng lại bản đã tải.

## Dùng
1. Maya: chọn mesh high-poly → **Open viewer (selected mesh)**. (Bấm lại sau khi sửa mesh: viewer tự nạp lại.)
2. Viewer:
   - kéo chuột trái: vẽ nét lên mesh; click vào nét: chọn / bỏ chọn (nét chọn màu vàng)
   - RMB hoặc Alt+LMB: xoay · MMB: pan · cuộn: zoom · Ctrl+Z: hủy nét · Del: xóa nét đang chọn
   - chọn 2 nét → **Grid from 2 strokes**; chọn 4 nét khép kín → **Grid from 4 strokes** (U, V = số span)
   - **Symmetry** (X/Y/Z + vị trí mặt phẳng, `plane = center` lấy tâm mesh): nét vẽ tự có nét đối xứng; Grid from 2/4 strokes tạo luôn patch đối xứng
   - **Auto patch**: vẽ một mạng nét cắt nhau (hoặc chạm nhau) → mỗi ô 4 cạnh thành một patch; *edge* = kích thước quad mong muốn (% kích thước mesh), *touch* = khoảng cách để coi hai nét là giao nhau. Các ô kề nhau luôn khớp số span nên hàn khít. Chọn ≥2 nét thì chỉ dùng các nét đó. Ô 3/5 cạnh bị bỏ qua.
   - **Export retopo.obj** (hàn các patch sát nhau theo ngưỡng *weld*)
3. Maya: **Import retopo mesh**.

Mẹo: hai patch kề nhau chỉ hàn khít khi nét chung được chia cùng số span (đặt U = V cho đơn giản).
Đơn vị scene Maya nên là cm. Raycast brute-force: mesh > ~500k tris sẽ chậm.

## Phát hành viewer (người làm tool)
Chuyển `ci/release.yml` vào `.github/workflows/release.yml`, push, rồi:
```
git tag v0.1.0 && git push origin v0.1.0
```
GitHub Actions build Windows/macOS/Linux và gắn file vào Release; script Maya tải bản `latest`.

## Dev
```
cd rust && cargo run --release     # cần Rust
cargo test
```

## Vẽ nhanh

- **Vòng tròn / elip:** vẽ tay một vòng (hoặc cung quanh ~60% trở lên), viewer tự khép thành vòng chuẩn nằm trên bề mặt. Tắt ở tab STROKES. Hai vòng được chọn rồi *Grid from 2* sẽ nối thành ống (tay, chân, ngón).
- **Shift + kéo:** kẻ đường thẳng.
- **Tự lưu:** stroke, patch và thiết lập lưu cạnh `mesh.obj` (`session_*.rtz`), mở lại viewer là có lại; Maya export lại cùng mesh thì giữ nguyên việc đang làm.
- Phím: G grid, A auto, M mirror, E export, P ẩn/hiện sổ, Ctrl+Z, Del.

Font nhúng: Cinzel, Caveat, Oswald, IBM Plex Mono (SIL OFL), Special Elite (Apache 2.0), giấy phép trong `rust/assets/`.

## Chạy độc lập, không cần Maya

Viewer là một file exe riêng. Mở bằng cách:
- kéo thả file `.obj` vào cửa sổ viewer;
- hoặc chuột phải file `.obj` → Open with → `retopoez.exe`, hoặc kéo file lên exe (viewer đang mở thì nó nhận file và hiện lên, không mở cửa sổ thứ hai);
- hoặc `retopoez.exe đường\dẫn\mesh.obj`.

Lần sau mở viewer không kèm file, nó mở lại file lần trước. Export (phím E) lưu `<tên>_retopo.obj` cạnh file gốc; import vào Maya hay phần mềm khác bằng File → Import. Cầu nối Maya (`retopoez.py`) vẫn dùng được nhưng không bắt buộc.

## Phím tắt chính
Space (giữ) = menu radial · Tab (bấm) = bật/tắt Edit chọn mặt, Tab (giữ) = Edit tạm thời rồi thả về pencil · Shift+kéo = đường thẳng · R relax · G grid · A auto · L live · M mirror · E export · Ctrl+Z.
Edit: click chọn mặt, click lại để bỏ chọn, click chỗ trống để bỏ hết (không quét khung). Giữ Tab + click để chọn nhanh rồi thả Tab về pencil.
