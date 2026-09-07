(function (root) {
  'use strict';
  const guides = {
    serverchan: {
      url: 'https://sct.ftqq.com/',
      steps: [
        ['打开 Server酱 Turbo，登录后按平台提示绑定消息接收端。','Sign in to ServerChan Turbo and bind a recipient using its setup flow.','Войдите в ServerChan Turbo и привяжите получателя по инструкции платформы.','سجّل الدخول إلى ServerChan Turbo واربط جهة الاستقبال وفق خطوات المنصة.'],
        ['在 SendKey 页面复制 SCT 开头的密钥，粘贴到左侧 SendKey。无需填写 Webhook URL。','Copy the SCT-prefixed key from the SendKey page into SendKey. No Webhook URL is required.','Скопируйте ключ с префиксом SCT со страницы SendKey. URL Webhook не нужен.','انسخ المفتاح الذي يبدأ بـ SCT من صفحة SendKey إلى الحقل المقابل. لا يلزم رابط Webhook.'],
        ['启用此渠道和总转发开关，保存后点击“测试推送”，确认接收端收到消息。','Enable this channel and forwarding, save, then send a test notification and check the recipient.','Включите канал и пересылку, сохраните и отправьте тестовое уведомление.','فعّل القناة وإعادة التوجيه واحفظ، ثم أرسل إشعاراً اختبارياً وتحقق من وصوله.']
      ],
      note: ['SendKey 留空保留已存密钥。平台推送额度和套餐限制以 Server酱为准；此处仅支持 Turbo。','A blank SendKey keeps the saved key. Turbo only; delivery quotas follow your ServerChan plan.','Пустое поле сохраняет ключ. Поддерживается только Turbo; действуют лимиты вашего тарифа.','ترك SendKey فارغاً يحتفظ بالمفتاح المحفوظ. يدعم Turbo فقط وتطبق حدود باقة المنصة.']
    },
    wecom: {
      url: 'https://developer.work.weixin.qq.com/document/path/91770',
      steps: [
        ['在企业微信群中添加群机器人，复制机器人提供的完整 Webhook 地址。','Add a group robot in WeCom and copy its complete Webhook URL.','Добавьте группового робота в WeCom и скопируйте полный URL Webhook.','أضف روبوتاً إلى مجموعة WeCom وانسخ رابط Webhook كاملاً.'],
        ['粘贴以 https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key= 开头的地址，保留 key 参数。','Paste the URL starting with https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=, including the key.','Вставьте адрес https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key= вместе с ключом.','ألصق الرابط الذي يبدأ بـ https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key= مع قيمة المفتاح.'],
        ['启用并保存渠道，然后测试推送。群里应出现包含设备名称、号码、时间和短信正文的文本消息。','Enable and save the channel, then test. The group receives the device, sender, time and message text.','Включите и сохраните канал, затем проверьте получение названия устройства, номера, времени и текста.','فعّل القناة واحفظ ثم اختبرها. ستصل للمجموعة رسالة تتضمن الجهاز والمرسل والوقت والنص.']
      ],
      note: ['使用群机器人，不需要企业 ID 或应用 Secret。Webhook 留空可保留原地址；机器人限流时最多重试 3 分钟。','Uses a group robot, not an enterprise application. Blank URL keeps the saved URL. Retries last up to 3 minutes.','Используется групповой робот; ID предприятия и Secret приложения не нужны. Повторы до 3 минут.','تستخدم هذه القناة روبوت المجموعة ولا تحتاج معرّف المؤسسة أو سر التطبيق. تستمر المحاولات حتى 3 دقائق.']
    },
    dingtalk: {
      url: 'https://open.dingtalk.com/',
      steps: [
        ['在钉钉群的机器人管理中添加“自定义”机器人，复制带 access_token 的完整 Webhook。','Add a Custom robot in the DingTalk group and copy its complete Webhook with access_token.','Добавьте пользовательского робота в группу DingTalk и скопируйте Webhook с access_token.','أضف روبوتاً مخصصاً في مجموعة DingTalk وانسخ رابط Webhook كاملاً مع access_token.'],
        ['若机器人开启“加签”，将 SEC 开头的签名密钥填入“签名密钥”；未开启则留空。','If signature verification is enabled, enter its SEC-prefixed signing secret. Otherwise leave it blank.','Если включена проверка подписи, введите ключ с префиксом SEC; иначе оставьте поле пустым.','إذا كان التحقق بالتوقيع مفعّلاً فأدخل السر الذي يبدأ بـ SEC، وإلا فاتركه فارغاً.'],
        ['若使用关键词校验，将关键词设为 SMS；若使用 IP 白名单，填写模块实际公网出口 IP。保存后测试推送。','For keyword validation use SMS. For IP restrictions allow the module’s public egress IP. Save and test.','Для проверки ключевого слова используйте SMS; в IP-списке разрешите внешний адрес модуля. Сохраните и проверьте.','عند استخدام الكلمات المفتاحية اختر SMS، وعند تقييد IP اسمح بعنوان الخروج العام للوحدة. احفظ واختبر.']
      ],
      note: ['加签失败时检查模块时间及密钥；关键词/IP 限制必须与机器人安全设置一致。','For signing errors check the module clock and secret. Keyword/IP restrictions must match the robot settings.','При ошибке подписи проверьте время модуля и ключ. Ограничения должны совпадать с настройками робота.','عند فشل التوقيع تحقق من ساعة الوحدة والسر. يجب أن تطابق قيود الكلمات وIP إعدادات الروبوت.']
    },
    feishu: {
      url: 'https://open.feishu.cn/',
      steps: [
        ['在飞书群设置中添加自定义机器人，复制 /open-apis/bot/v2/hook/ 开头路径的 Webhook。','Add a custom bot in the Feishu group and copy the Webhook containing /open-apis/bot/v2/hook/.','Добавьте пользовательского бота в группу Feishu и скопируйте Webhook с /open-apis/bot/v2/hook/.','أضف روبوتاً مخصصاً في مجموعة Feishu وانسخ رابط Webhook الذي يحتوي على /open-apis/bot/v2/hook/.'],
        ['机器人启用签名校验时，将对应密钥填入“签名密钥”。关键词校验可使用 SMS。','Enter the signing secret if the bot requires signature verification. SMS can be used for keyword validation.','Если бот проверяет подпись, введите его ключ. Для проверки ключевого слова подходит SMS.','أدخل سر التوقيع إذا كان الروبوت يتحقق من التوقيع. يمكن استخدام SMS للتحقق بالكلمات المفتاحية.'],
        ['启用并保存渠道，点击测试推送，在该群确认文本消息。若限制 IP，允许模块实际公网出口 IP。','Enable, save and test the channel. If IP restrictions apply, allow the module’s public egress IP.','Включите, сохраните и проверьте канал. При ограничении IP разрешите внешний адрес модуля.','فعّل القناة واحفظ واختبر وصول الرسالة. إذا وُجد تقييد IP فاسمح بعنوان الخروج العام للوحدة.']
      ],
      note: ['这是群自定义机器人，不需要自建应用的 App ID / App Secret；签名需要模块时间准确。','This is a group custom bot; no App ID or App Secret is required. Signing requires an accurate module clock.','Это групповой бот: App ID и App Secret не нужны. Для подписи необходимо точное время модуля.','هذا روبوت مجموعة مخصص ولا يحتاج App ID أو App Secret. يتطلب التوقيع ساعة صحيحة على الوحدة.']
    },
    webhook: {
      url: '',
      steps: [
        ['填写你自己的 HTTP/HTTPS 接收地址，接口需要接受 POST application/json，消息字段见下方示例。','Enter your HTTP/HTTPS endpoint. It must accept POST application/json with the fields shown below.','Укажите HTTP/HTTPS-адрес, принимающий POST application/json с полями из примера ниже.','أدخل عنوان HTTP/HTTPS الذي يقبل POST بنوع application/json والحقول المبينة أدناه.'],
        ['需要鉴权时填写完整 Authorization 值，例如 Bearer your-token；无需鉴权则留空。','For authentication enter the full Authorization value, for example Bearer your-token. Otherwise leave it blank.','Для авторизации введите полное значение Authorization, например Bearer your-token; иначе оставьте пустым.','عند الحاجة للمصادقة أدخل قيمة Authorization كاملة مثل Bearer your-token، وإلا فاتركها فارغة.'],
        ['接收成功返回任意 HTTP 2xx。按 Idempotency-Key 请求头去重；长短信通过 part/parts 分段，按 id 和 part 合并。','Return HTTP 2xx on success. Deduplicate by Idempotency-Key; reassemble long messages using id, part and parts.','Верните HTTP 2xx при успехе. Удаляйте дубли по Idempotency-Key; объединяйте части по id, part и parts.','أعد HTTP 2xx عند النجاح. امنع التكرار باستخدام Idempotency-Key واجمع الرسائل الطويلة وفق id وpart وparts.']
      ],
      note: ['不跟随重定向。连接失败、408、429、5xx 最多重试 3 分钟，其他非 2xx 视为失败；text 为正文分段。','Redirects are not followed. Connection errors, 408, 429 and 5xx retry for up to 3 minutes. Other non-2xx responses fail. text contains one message part.','Перенаправления не выполняются. Ошибки связи, 408, 429 и 5xx повторяются до 3 минут. text содержит часть сообщения.','لا يتم اتباع إعادة التوجيه. تعاد محاولات أخطاء الاتصال و408 و429 و5xx حتى 3 دقائق. يحتوي text على جزء من الرسالة.']
    },
    sim: {
      url: '',
      steps: [
        ['确认本卡已开通短信发送服务且余额充足；此渠道使用模块内 SIM 卡发送，不依赖 Webhook。','Ensure the SIM can send SMS and has sufficient credit. This channel sends through the module’s SIM, without a Webhook.','Убедитесь, что SIM может отправлять SMS и на счёте достаточно средств. Webhook не используется.','تأكد من تفعيل إرسال SMS على الشريحة وكفاية الرصيد. تستخدم القناة شريحة الوحدة دون Webhook.'],
        ['填写一个接收号码，建议使用 +国家码格式，例如 +8613800138000。不要填写本卡号码，也不要让接收端再转回本卡。','Enter one destination, preferably in +country-code format, e.g. +8613800138000. Do not use this SIM’s number or forward messages back to it.','Введите один номер в формате +код страны, например +8613800138000. Не указывайте номер этой SIM и не пересылайте сообщения обратно.','أدخل رقم استقبال واحداً بصيغة +رمز الدولة، مثل +8613800138000. لا تستخدم رقم هذه الشريحة ولا تعِد الرسائل إليها.'],
        ['启用本渠道、总转发和短信服务后保存。之后仅转发新收到的短信，内容包含原号码、接收时间、设备名称和正文。','Enable this channel, forwarding and SMS service, then save. Only newly received messages are forwarded, including sender, time, device and text.','Включите канал, пересылку и службу SMS, затем сохраните. Пересылаются только новые SMS с номером, временем, устройством и текстом.','فعّل القناة وإعادة التوجيه وخدمة SMS ثم احفظ. تُعاد توجيه الرسائل الجديدة فقط مع المرسل والوقت والجهاز والنص.']
      ],
      note: ['长短信会分段并可能产生多条费用。本渠道不自动发送测试短信，发送未获确认时不重试；来自目标号码的短信不再转回该号码，避免循环。成功表示运营商接受提交，不代表对方已读。','Long messages may incur multiple SMS charges. This channel has no automatic test and does not retry unconfirmed sends. Messages from the destination are not sent back. Success means submission accepted, not read by the recipient.','Длинные сообщения оплачиваются по частям. Автотест и повтор неподтверждённой отправки отключены. SMS от получателя не пересылаются обратно. Успех означает принятие отправки, а не прочтение.','قد تُحتسب الرسائل الطويلة عدة رسائل مدفوعة. لا يوجد اختبار تلقائي ولا إعادة للإرسال غير المؤكد. لا تُعاد رسائل الوجهة إليها. النجاح يعني قبول الإرسال وليس قراءة المستلم.']
    }
  };
  root.ForwardingGuides = { get(platform, language) {
    const guide=guides[platform] || guides.webhook;
    const index=Math.max(0,['zh-CN','en','ru','ar'].indexOf(language));
    return {url:guide.url,steps:guide.steps.map(row=>row[index]),note:guide.note[index]};
  }};
})(window.SimpleAdmin);
