use std::fs::File;
use std::io::{self, BufWriter};
use prettytable::{Table, Row, Cell, format};
use crate::args::Args;
use crate::PingResult;
use crate::common;

macro_rules! table_headers {
    () => {
        ["IP 地址", "已发送", "已接收", "丢包率", "平均延迟", "下载速度 (MB/s)", "数据中心"]
    };
}

/// 从 PingResult 导出 CSV 文件
pub fn export_csv(results: &[PingResult], args: &Args) -> io::Result<()> {
    if results.is_empty() || args.output.is_empty() {
        return Ok(());
    }

    let file = File::create(&args.output)?;
    let mut writer = csv::Writer::from_writer(BufWriter::with_capacity(32 * 1024, file));

    // 写入表头
    writer.write_record(&table_headers!())?;

    // 写入数据
    for result in results {
        // 使用模式匹配处理不同类型的结果
        match result {
            PingResult::Http(data) if args.httping => {
                let record = common::ping_data_to_csv_record(data);
                writer.write_record(&record)?;
            },
            PingResult::Tcp(data) if !args.httping && !args.icmp_ping => {
                let record = common::ping_data_to_csv_record(data);
                writer.write_record(&record)?;
            },
            PingResult::Icmp(data) if args.icmp_ping => {
                let record = common::ping_data_to_csv_record(data);
                writer.write_record(&record)?;
            },
            _ => {} // 忽略不匹配的情况
        }
    }

    writer.flush()?;
    Ok(())
}

/// 定义结果打印 trait
pub trait PrintResult {
    fn print(&self, args: &Args, no_qualified: bool);
}

/// 为 Vec<PingResult> 实现 PrintResult trait
impl PrintResult for Vec<PingResult> {
    /// 实现结果打印功能
    fn print(&self, args: &Args, no_qualified: bool) {
        if self.is_empty() {
            println!("\n[信息] 完整测速结果 IP 数量为 0，跳过输出结果");
            return;
        }

        if no_qualified {
            println!("\n[信息] 下载测速结果没有达到所需数量，返回全部测速结果");
        }

        let mut table = Table::new();
        
        // 设置表格样式
        table.set_format(*format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
        
        // 添加表头，使用青色
        table.add_row(Row::new(
            table_headers!().iter()
                .map(|&h| Cell::new(h).style_spec("Fc"))
                .collect::<Vec<_>>()
        ));

        // 添加数据行，最多显示 args.print_num 条
        for result in self.iter().take(args.print_num.into()) {
            // 使用模式匹配处理不同类型的结果
            match result {
                PingResult::Http(data) if args.httping => {
                    table.add_row(common::ping_data_to_table_row(data));
                },
                PingResult::Tcp(data) if !args.httping && !args.icmp_ping => {
                    table.add_row(common::ping_data_to_table_row(data));
                },
                PingResult::Icmp(data) if args.icmp_ping => {
                    table.add_row(common::ping_data_to_table_row(data));
                },
                _ => {} // 忽略不匹配的情况
            }
        }

        // 打印表格
        table.printstd();

        // 如果有输出文件，打印提示
        if !args.output.is_empty() {
            println!("\n[信息] 完整测速结果已写入 {} 文件，可使用记事本/表格软件查看", args.output);
        }
    }
}