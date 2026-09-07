using System;
using System.Collections.Generic;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace SimpleAdminSetup
{
    static class InstallOptions
    {
        public static string Credentials(bool web, string username, string password, bool root, string rootPassword)
        {
            if (!web && !root) return null;
            var fields = new Dictionary<string, string>();
            void Validate(string value) {
                if (string.IsNullOrEmpty(value) || Encoding.UTF8.GetByteCount(value) > 128 || value.IndexOfAny(new[] { '\r', '\n', '\0' }) >= 0)
                    throw new ArgumentException("密码需为 1–128 字节，不能包含换行或空字符；中文字符通常占 3 字节。");
            }
            if (web) {
                if (username == null || !Regex.IsMatch(username, @"\A[A-Za-z0-9_.-]{1,64}\z"))
                    throw new ArgumentException("Web 账号需为 1–64 位字母、数字、点、下划线或短横线。");
                Validate(password);
                if (password.Trim() != password) throw new ArgumentException("Web 密码的开头和结尾不能使用空白字符。");
                fields["web_username"] = username;
                fields["web_password"] = password;
            }
            if (root) { Validate(rootPassword); fields["root_password"] = rootPassword; }
            return JsonSerializer.Serialize(fields);
        }
    }
}