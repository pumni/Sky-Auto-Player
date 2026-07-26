# 2026-07-26 Pages Indexing Runbook Plan

## Mục tiêu
Đây là kế hoạch cải tiến deployment/indexing residual sau plan SEO đã được shipped. Kế hoạch quản lý cấu hình GitHub Pages artifact và quá trình human rollout để đảm bảo SEO/indexing an toàn.
- Kế hoạch hiện tại không ghi đè cấu trúc updater hay tương thích ứng dụng mà các plan cũ đã vạch ra (chẳng hạn như update compatibility bridge).
- Kế hoạch hướng dẫn quá trình deploy Pages thông qua GitHub Actions allowlist thay vì commit file tĩnh vào Pages default workflow, ngăn chặn tài liệu kĩ thuật của repo leak ra ngoài và tránh nhầm lẫn indexing.

## Chi tiết triển khai

1. **Build Allowlist (`scripts/build_pages_site.ps1`)**: Script build file static site (từ `docs/`) chỉ copy đúng các asset marketing, `.nojekyll`, file xác minh `google*.html`, `sitemap.xml`, và các file cần thiết khác.
2. **Validator (`scripts/validate_pages_site.ps1`)**: Script sẽ kiểm tra cấu trúc schema, alternate URLs, và thành phần allowlist được sinh ra. Kiểm tra version động qua `pyproject.toml`.
3. **Workflow Pages (`.github/workflows/pages.yml`)**: Tạo job build & deploy thông qua GitHub actions cho nhánh `main`. Không deploy trên PR.
4. Giữ đúng bốn canonical URL:
   - `/Sky-Auto-Player/`
   - `/Sky-Auto-Player/vi/`
   - `/Sky-Auto-Player/faq.html`
   - `/Sky-Auto-Player/vi/faq.html`

## Ghi chú
- `AGENTS.md` luôn thắng khi là agent instruction.
- Các normative architecture docs thắng plan này.
- Kế hoạch migration updater cũ vẫn là nguồn chuẩn cho compatibility.
