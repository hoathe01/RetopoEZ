# RetopoEZ: kế hoạch

Viewer Rust + wgpu (eframe/egui) vẽ stroke lên mesh high-poly, sinh lưới quad bám bề mặt, gửi về Maya qua OBJ. Maya chỉ có hai nút: Open viewer, Import retopo mesh.

## Đã có (v0.1)

- Stroke vẽ tay trên bề mặt, chọn stroke, undo, xoá.
- Grid từ 2 stroke (bridge) và 4 stroke (Coons), dính bề mặt, weld khi export.
- Nhận diện vòng tròn / elip, nối hai vòng thành ống. Shift kéo ra đường thẳng.
- Symmetry theo trục X/Y/Z, stroke đối xứng hiện ngay khi đang vẽ.
- Smart Auto: bật LIVE thì patch tự dựng lại sau mỗi stroke; đầu stroke tự dính vào stroke gần nhất (16 px); FREEZE để chốt thành patch thường.
- Low mesh: tô mặt xanh biển đậm, cạnh sáng hơn, vẽ đè lên high mesh. Đổi màu và độ mờ ở tab EXPORT.
- Relax brush (phím R): kéo trên low mesh để làm đều ô lưới, trượt theo bề mặt, giữ nguyên viền patch; Ctrl+Z hoàn tác từng nét.
- Menu tròn: giữ Tab, rê tới ô muốn chọn rồi thả (Draw, Relax, Grid, Live, Mirror, Export, Undo, Delete).
- Vẽ line bằng GPU (độ dày cố định theo pixel) và cache buffer: mỗi khung hình chỉ cập nhật uniform, chỉ dựng lại khi nét hoặc patch đổi.
- Tự lưu phiên làm việc cạnh `mesh.obj`; Maya export lại cùng mesh không mất việc.
- BVH cho raycast và closest point (hàng triệu tam giác vẫn mượt).
- Một cửa sổ viewer duy nhất; lần đầu chạy từ Maya tự tải viewer từ GitHub Releases.

## Việc cần làm ngay (P0)

- [ ] Chạy thật trên Windows, chụp màn hình, chỉnh màu, khoảng cách, cỡ chữ của giao diện Casebook.
- [ ] Kiểm tra Symmetry và Live Auto trên mesh thật (con ếch): đúng chiều, không lệch mặt phẳng.
- [ ] `git push`, chuyển `ci/release.yml` sang `.github/workflows/release.yml`, tag `v0.1.0`, thử Maya tải viewer.

## Chất lượng lưới (P1)

- [ ] Relax tự động sau khi dựng patch; brush cho phép kéo cả điểm viền.
- [ ] Vùng 3 cạnh và 5 cạnh thật sự (không có chỗ bẻ góc): tách thành 3 / 5 quad quanh một điểm giữa. Chỗ bẻ góc gắt trên stroke đã tự thành góc ô.
- [ ] T-junction (stroke kết thúc giữa stroke khác).
- [ ] Tự chọn kích thước ô theo độ cong mặt (cong nhiều thì ô nhỏ).
- [ ] Phát hiện stroke vòng khép kín nằm trong mạng auto (hiện xử lý như stroke thường).
- [ ] Symmetry: đổi mặt phẳng thì cập nhật lại các stroke đối xứng; mirror cả patch đã tạo bằng tay.

## Trải nghiệm (P2)

- [ ] Undo/redo hợp nhất cho stroke và patch (hiện Ctrl+Z chỉ lùi stroke, patch có nút riêng).
- [ ] Chọn nhiều stroke bằng khung kéo.
- [ ] Cập nhật từng phần buffer GPU khi relax (hiện dựng lại toàn bộ patch mỗi khung hình khi đang brush).
- [ ] Preview đường thẳng và đối xứng bám bề mặt ngay khi kéo, không chỉ khi thả.
- [ ] Hiện vùng chưa phủ và vùng auto bỏ qua để biết chỗ cần vẽ thêm.
- [ ] Đơn vị scene tự nhận từ Maya; hoàn tác thay đổi thông số.

## Tích hợp Maya (P3)

- [ ] Import retopo thành mesh mới có tên theo mesh gốc, giữ normals, tuỳ chọn gắn lên layer.
- [ ] Gửi từng phần về Maya (không cần export toàn bộ mỗi lần).
- [ ] Nhiều object high-poly cùng lúc; mesh toàn quad.
- [ ] Hỗ trợ macOS / Linux build trong CI và kiểm tra chữ ký tải về (checksum).
- [ ] Tự báo khi có bản viewer mới.

## Giới hạn đã biết

- Auto chỉ dựng ô 4 cạnh; các vùng khác bị bỏ qua.
- Nhận diện vòng tròn chỉ nhận khi vòng nằm trên bề mặt, không nhảy lớp (mi mắt và nhãn cầu) và bám sát nét vẽ; không thì giữ nguyên nét tay và báo ở thanh trạng thái.
- Mirror giả định mặt ngoài của mesh theo hướng pháp tuyến ra ngoài.
- Cửa sổ chưa thể chạy trên máy ảo không có GPU Vulkan/DX12.

## Cách kiểm tra

- `cd rust && cargo test`: hình học, BVH so với brute force, nhận diện vòng, session, auto patch.
- `cargo run --release`: chạy viewer với `mesh.obj` trong `%TEMP%\retopoez`, hoặc `cargo run --release -- path\to\mesh.obj`.
