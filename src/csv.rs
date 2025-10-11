use std::fs::File;
use std::io::{self, BufWriter};
use prettytable::{Table, Row, Cell, format};
use crate::args::Args;
use crate::common::{PingData, PingDataRef};

const TABLE_HEADERS: [&str; 7] = [
    "IP 地址", 
    "已发送", 
    "已接收", 
    "丢包率", 
    "平均延迟", 
    "下载速度(MB/s)", 
    "数据中心"
];


/// 定义结果打印 trait
pub trait PrintResult {
    fn print(&self, args: &Args);
}

/// 从 PingResult 导出 CSV 文件
pub fn export_csv(results: &[PingData], args: &Args) -> io::Result<()> {
    if results.is_empty() || args.output.is_none() {
        return Ok(());
    }

    let file = File::create(args.output.as_ref().unwrap())?;
    let mut writer = csv::Writer::from_writer(BufWriter::with_capacity(32 * 1024, file));

    // 写入表头
    writer.write_record(&TABLE_HEADERS)?;

    // 写入数据
    for result in results {
        let mut record = ping_data_to_fields(&result.as_ref());
        record[0] = result.as_ref().display_addr(args.show_port);
        writer.write_record(&record)?;
    }

    writer.flush()?;
    Ok(())
}

impl PrintResult for Vec<PingData> {
    /// 实现结果打印功能
    fn print(&self, args: &Args) {
        if self.is_empty() {
            println!("\n[信息] 测速结果 IP 数量为 0，跳过输出结果");
            return;
        }

        let mut table = Table::new();
        
        // 可选的表格格式（选择其中一种）：
        // * FORMAT_DEFAULT - 默认样式，带有边框和分隔线
        // * FORMAT_NO_BORDER - 无外部边框，但保留列和行的分隔线
        // * FORMAT_NO_BORDER_LINE_SEPARATOR - 无外部边框和行分隔线，仅保留列分隔线
        // * FORMAT_NO_COLSEP - 无列分隔符，仅保留行分隔线和边框
        // * FORMAT_NO_LINESEP - 无行分隔线和标题分隔线，仅保留列分隔符和边框
        // * FORMAT_NO_LINESEP_WITH_TITLE - 无行分隔线，但保留标题分隔线
        // * FORMAT_NO_TITLE - 类似于默认样式，但没有标题行下的特殊分隔线
        // * FORMAT_CLEAN - 无任何分隔符，仅保留内容对齐
        // * FORMAT_BORDERS_ONLY - 仅显示外部边框和标题分隔线
        // * FORMAT_BOX_CHARS - 使用盒字符（如 ┌─┬─┐）绘制边框和分隔线，适用于支持 Unicode 的终端
        table.set_format(*format::consts::FORMAT_CLEAN);
        
        // 添加表头，使用青色
        table.add_row(Row::new(
            TABLE_HEADERS.iter()
                .map(|&h| Cell::new(h).style_spec("Fc"))
                .collect::<Vec<_>>()
        ));

        // 添加数据行，最多显示 args.print_num 条
        for result in self.iter().take(args.print_num.into()) {
            let result_ref = result.as_ref();
            let first_cell = Cell::new(&result_ref.display_addr(args.show_port));
            let other_cells = ping_data_to_fields(&result_ref)
                .into_iter()
                .skip(1)
                .map(|field| Cell::new(&field));
            let row = Row::new(std::iter::once(first_cell).chain(other_cells).collect());
            table.add_row(row);
        }

        // 打印表格
        table.printstd();

        // 如果有输出文件，打印提示
        if let Some(ref output) = args.output {
            println!("\n[信息] 测速结果已写入 {} 文件，可使用记事本/表格软件查看", output);
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