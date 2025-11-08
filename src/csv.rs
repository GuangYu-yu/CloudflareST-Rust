use crate::args::Args;
use crate::common::{PingData, PingDataRef};
use crate::info_println;
use std::io::Write;

const TABLE_HEADERS: [&str; 7] = [
    "IP 地址",
    "已发送",
    "已接收",
    "丢包率",
    "平均延迟",
    "下载速度(MB/s)",
    "数据中心",
];

/// 定义结果打印 trait
pub trait PrintResult {
    fn print(&self, args: &Args);
}

/// 从 PingResult 导出 CSV 文件
pub fn export_csv(results: &[PingData], args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    /// 写入CSV行到文件
    fn write_csv_line(file: &mut std::fs::File, fields: &[String]) -> Result<(), Box<dyn std::error::Error>> {
        let line = fields.join(",");
        writeln!(file, "{}", line)?;
        Ok(())
    }

    // 如果没有结果或未指定输出文件，直接返回
    if results.is_empty() || args.output.as_ref().is_none() {
        return Ok(());
    }

    let file_path = args.output.as_ref().unwrap();
    let mut file = std::fs::File::create(file_path)?;

    // 写入表头
    write_csv_line(&mut file, &TABLE_HEADERS.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;

    // 写入数据
    for result in results {
        let mut record = ping_data_to_fields(&result.as_ref());
        record[0] = result.as_ref().display_addr(args.show_port);
        write_csv_line(&mut file, &record)?;
    }

    // 确保数据写入磁盘
    file.flush()?;

    Ok(())
}

impl PrintResult for Vec<PingData> {
    fn print(&self, args: &Args) {
        if self.is_empty() {
            info_println(format_args!("测速结果 IP 数量为 0，跳过输出结果"));
            return;
        }

        const COLUMN_PADDING: usize = 3; // 每列额外间距
        const LEADING_SPACES: usize = 1; // 前导空格数量

        let print_num = self.len().min(args.print_num.into());
        let header_display_widths: [usize; 7] = [7, 6, 6, 6, 8, 14, 8]; 
        let mut column_widths = header_display_widths.to_vec();

        // 预先计算每行字段显示值，并更新列宽
        let rows: Vec<Vec<String>> = self.iter()
            .take(print_num)
            .map(|result| {
                let r = result.as_ref();
                ping_data_to_fields(&r)
                    .into_iter()
                    .enumerate()
                    .map(|(i, field)| {
                        let display_field = if i == 0 { r.display_addr(args.show_port) } else { field };
                        let width = display_field.chars().count();
                        if width > column_widths[i] {
                            column_widths[i] = width;
                        }
                        display_field
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        // 打印表头
        let header_row: String = " ".repeat(LEADING_SPACES) + &TABLE_HEADERS
            .iter()
            .enumerate()
            .map(|(i, header)| {
                let pad = column_widths[i].saturating_sub(header_display_widths[i]) + COLUMN_PADDING;
                format!("{}{}", header, " ".repeat(pad))
            })
            .collect::<String>();
        // 打印表头 (使用亮白色+粗体)
        println!("\x1b[1;97m{}\x1b[0m", header_row);

        // 打印数据行
        for row in rows {
            let row_str: String = " ".repeat(LEADING_SPACES) + &row
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let pad = column_widths[i].saturating_sub(field.chars().count()) + COLUMN_PADDING;
                    format!("{}{}", field, " ".repeat(pad))
                })
                .collect::<String>();
            println!("{}", row_str);
        }
    }
}

/// 将 PingDataRef 转换为通用数据格式
fn ping_data_to_fields(data: &PingDataRef) -> Vec<String> {
    vec![
        data.addr.to_string(),
        data.sent.to_string(),
        data.received.to_string(),
        format!("{:.2}", data.loss_rate()),
        format!("{:.2}", data.delay),
        match data.download_speed {
            Some(speed) => format!("{:.2}", speed / 1024.0 / 1024.0),
            None => String::new(),
        },
        data.data_center.to_string(),
    ]
}