# RetopoEZ: kế hoạch

Viewer Rust + wgpu (eframe/egui) vẽ stroke lên mesh high-poly, sinh lưới quad bám bề mặt. Chạy độc lập (kéo thả .obj); Maya chỉ là cầu nối tuỳ chọn. Mục tiêu dài hạn: retopo **lười nhất có thể**, ít thao tác nhất cho một mesh dùng được.

## Đã có

- Stroke vẽ tay trên bề mặt; tự làm mượt nét gãy khi nhả chuột (giữ góc nhọn, nét gần thẳng thành thẳng); nhận diện vòng/elip; Shift = đường thẳng.
- Grid từ 2 stroke (bridge) và 4 stroke (Coons); Smart Auto + LIVE dựng patch theo stroke; weld khi export.
- Symmetry thời gian thực cho stroke, brush (Relax, Grab) và chọn mặt.
- Low mesh xanh biển đậm vẽ đè lên high mesh.
- Brush (B hoặc 1): Relax (R) và Grab (V); size và hardness kiểu Substance Painter (`[` `]`, Ctrl + chuột phải kéo); Ctrl+Z hoàn tác từng nét.
- Tab: bấm = bật/tắt Edit, giữ = Edit tạm thời (giữ chuột rồi rê qua các mặt để chọn, Ctrl = bỏ chọn), thả Tab về pencil. Space giữ = menu tròn.
- Chạy độc lập, tự mở lại file cũ, export `<tên>_retopo.obj`, tự lưu phiên, BVH, GPU line.

## Học từ Cozy Blanket (phần retopo)

Trang tính năng của Cozy Blanket có 18 công cụ cử chỉ để dựng lưới thấp. PathD là một trong số đó: nét vẽ sinh ra các vertex cách đều nhau dọc đường ngắn nhất giữa vertex đầu và cuối. Ánh xạ sang RetopoEZ:

| Công cụ | Ý nghĩa | Trạng thái |
|---|---|---|
| PathD | Rải vertex đều dọc đường ngắn nhất giữa hai điểm | **Làm tiếp, xem dưới** |
| Relax (tự ghim góc) | Làm đều lưới, góc cố định | Có; thêm ghim góc tự động |
| Move | Kéo lưới, ảnh hưởng lan theo bề mặt | Có (Grab) |
| PinFlip | Ghim vertex dưới nét vẽ, không bị Relax/Grab đụng | Chưa |
| Erase | Xoá mặt dưới nét vẽ | Chưa (cần sửa topology) |
| BuildQ / StripD | Kéo từ cạnh ra quad / dải quad | Chưa, ưu tiên cao |
| ExtendB | Chọn cạnh biên rồi kéo thành dải mới | Chưa |
| MergeP | Nối hai vertex hoặc hai tam giác kề | Chưa |
| SurfCut | Cắt ngang mặt tạo cạnh mới | Chưa |
| Clone | Chép patch sang vùng khác | Chưa |
| TransfV, Tweak | Dời vertex hoặc loop | Một phần (Grab) |

### Kế hoạch PathD (stroke thành chuỗi điểm đều)

1. Mô hình dữ liệu mới: **EditMesh** thật sự (vertex, cạnh, mặt, kề nhau) thay cho patch dạng lưới. Mọi công cụ bên dưới cần nó; đây là việc nền.
2. PathD: lấy mặt cắt đầu và cuối stroke, tính đường đi ngắn nhất trên bề mặt (geodesic; xấp xỉ bằng Dijkstra trên cạnh high-poly rồi làm mượt), rải N vertex đều. N lấy từ độ dài chia kích thước ô mục tiêu.
3. Nối với các điểm có sẵn: nếu đầu/cuối gần một vertex EditMesh thì dùng chung (như snap đầu stroke hiện tại).
4. Hai hàng PathD song song thành dải quad (StripD); đóng vòng thì thành ống.
5. Brush Relax và Grab chạy trực tiếp trên EditMesh (hiện chạy trên Weld của patch).

## Kế hoạch lười nhất có thể (theo thứ tự lời nhất)

Thang đo: số thao tác chuột cho một vùng retopo xong.

1. **Một nét ra cả dải** (StripD): kéo từ cạnh biên ra ngoài, tool tự đoán độ rộng bằng ô liền kề, bám bề mặt, tự relax.
2. **Chọn mặt rồi làm việc hàng loạt** (Edit hiện đã chọn được): Relax vùng chọn, xoá, nối vùng chọn thành patch, làm đều kích thước ô, extrude biên.
3. **Điền lỗ một cú click**: click vào vùng trống bao quanh bởi cạnh thì tự lấp quad (vùng 3 và 5 cạnh dùng điểm giữa).
4. **Auto-quad vùng chọn** (tuỳ chọn): bôi vùng, chạy QuadriFlow hoặc Instant Meshes (miễn phí, CPU) cho vùng đó, snap về mặt; người vẽ chỉ chỉnh biên. Chạy như tiến trình ngoài, không nhét vào viewer.
5. **Gợi ý hướng loop**: tính hướng cong chính trên high mesh, vẽ mờ trước khi người dùng vẽ; nét vẽ tự bị hút theo hướng đó khi sát nghĩa.
6. **Tự chọn kích thước ô theo độ cong** và đếm điểm cực (cực 3, cực 5) để báo chỗ xấu.
7. **Mặc định symmetry theo bản lề của mesh**: tự đoán mặt phẳng đối xứng (PCA + kiểm tra khớp) và đề nghị bật.
8. **Làm lại một vùng mà không mất việc**: undo/redo thống nhất cho stroke, patch, brush.

## Việc cần làm ngay

- [ ] Chạy thật trên Windows, xem lại giao diện mới (panel phẳng) và độ mượt Edit/Brush; chụp màn hình.
- [ ] Brush trên mesh lớn: đo thời gian mỗi khung hình; nếu chậm thì chỉ dựng lại vùng đổi.
- [ ] `git push`, chuyển `ci/release.yml` sang `.github/workflows/release.yml`, tag `v0.1.0` để Maya tải được viewer.
- [ ] EditMesh (xem trên), rồi PathD, rồi StripD.

## Giới hạn đã biết

- Auto chỉ dựng ô 4 cạnh; vùng 3 và 5 cạnh thật sự bị bỏ qua.
- Chọn mặt chưa có hành động đi kèm; Edit hiện chỉ chọn và tô sáng.
- Grab và Relax chạy trên patch đã weld; patch chồng lên nhau không tự nối.
- Mirror giả định mặt ngoài của mesh theo hướng pháp tuyến ra ngoài.
- Cửa sổ chưa thể chạy trên máy ảo không có GPU Vulkan/DX12.

## Cách kiểm tra

- `cd rust && cargo test`: hình học, BVH, nhận diện vòng, làm mượt nét, Weld/Relax/Grab, session, shader.
- `cargo run --release -- path\to\mesh.obj`.
