use super::Report;
use ratatui::{prelude::*, widgets::*};

#[derive(Default)]
pub struct View {
    pub report: Option<Report>,
    pub loading: bool,
    pub scroll: u16,
}

impl View {
    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let content = match &self.report {
            Some(report) => format!(
                "{}\n{}",
                if self.loading {
                    "正在重新采集；下方为旧报告"
                } else {
                    "r 只读刷新 · g 显式联网探测 github.com"
                },
                report.render()
            ),
            None if self.loading => "正在后台采集只读诊断；不会启动或重启服务…".into(),
            None => "尚未采集诊断。r 只读检查；g 向 github.com 发起真实 HTTPS 探测。".into(),
        };
        frame.render_widget(
            Paragraph::new(content)
                .block(Block::bordered().title(" 服务器诊断 · 一次采样，不是持续监控 "))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            area,
        );
    }
}
