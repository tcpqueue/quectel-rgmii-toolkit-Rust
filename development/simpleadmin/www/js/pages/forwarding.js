(function (global) {
  'use strict';
  const root = global.SimpleAdmin;
  const words = {
    '启用转发': ['Enable forwarding', 'Включить пересылку', 'تفعيل إعادة التوجيه'],
    '设备名称': ['Device name', 'Название устройства', 'اسم الجهاز'],
    '所有新短信': ['All new messages', 'Все новые SMS', 'جميع الرسائل الجديدة'],
    '重试上限': ['Retry window', 'Повторы в течение', 'مدة إعادة المحاولة'],
    '分钟': ['min', 'мин', 'دقائق'],
    '等待推送': ['Queued', 'В очереди', 'في الانتظار'],
    '运行中': ['Running', 'Работает', 'قيد التشغيل'],
    '等待首次读取': ['Initializing', 'Инициализация', 'جارٍ التهيئة'],
    '已停用': ['Disabled', 'Отключено', 'معطّل'],
    '已启用': ['Enabled', 'Включено', 'مفعّل'],
    '推送平台': ['Channel', 'Канал', 'القناة'],
    '签名密钥': ['Signing secret', 'Ключ подписи', 'مفتاح التوقيع'],
    '可选': ['Optional', 'Необязательно', 'اختياري'],
    '测试推送': ['Test notification', 'Тест уведомления', 'اختبار الإشعار'],
    '推送中…': ['Sending…', 'Отправка…', 'جارٍ الإرسال…'],
    '清除凭据': ['Clear credentials', 'Очистить ключи', 'مسح بيانات الاعتماد'],
    '待清除': ['Removal pending', 'Ожидает удаления', 'في انتظار المسح'],
    '近期推送': ['Recent notifications', 'Последние уведомления', 'الإشعارات الأخيرة'],
    '最近 50 条': ['Last 50 results', 'Последние 50 результатов', 'آخر 50 نتيجة'],
    '时间': ['Time', 'Время', 'الوقت'],
    '发送号码': ['Sender', 'Отправитель', 'المرسل'],
    '结果': ['Result', 'Результат', 'النتيجة'],
    '暂无推送记录': ['No notifications yet', 'Уведомлений пока нет', 'لا توجد إشعارات بعد'],
    '测试成功': ['Test delivered', 'Тест доставлен', 'تم إرسال الاختبار'],
    'sent': ['Delivered', 'Доставлено', 'تم التسليم'],
    'retrying': ['Retrying', 'Повторная попытка', 'إعادة المحاولة'],
    'failed': ['Failed', 'Ошибка', 'فشل'],
    'test': ['Test notification', 'Тестовое уведомление', 'إشعار اختبار'],
    'SMS read failed': ['SMS read failed', 'Ошибка чтения SMS', 'تعذرت قراءة الرسائل'],
    'retry window expired': ['Retry window expired', 'Время повторов истекло', 'انتهت مدة إعادة المحاولة'],
    'queue capacity exceeded': ['Queue is full', 'Очередь заполнена', 'قائمة الانتظار ممتلئة'],
    'network or TLS request failed': ['Connection failed', 'Ошибка подключения', 'فشل الاتصال'],
    'enable at least one channel': ['Enable at least one channel', 'Включите хотя бы один канал', 'فعّل قناة واحدة على الأقل'],
    'invalid ServerChan Turbo SendKey': ['Invalid ServerChan Turbo SendKey', 'Неверный SendKey ServerChan Turbo', 'مفتاح ServerChan Turbo غير صالح'],
    'invalid webhook URL': ['Invalid Webhook URL', 'Неверный URL Webhook', 'رابط Webhook غير صالح'],
    'webhook does not match selected platform': ['Webhook does not match the channel', 'Webhook не соответствует каналу', 'الرابط لا يتطابق مع القناة'],
    'webhook token missing': ['Webhook token missing', 'Отсутствует токен Webhook', 'رمز Webhook مفقود'],
    'invalid device name': ['Invalid device name', 'Неверное имя устройства', 'اسم الجهاز غير صالح'],
    'wait before testing again': ['Wait before testing again', 'Подождите перед новым тестом', 'انتظر قبل إعادة الاختبار']
  };
  const chinese = {sent:'已送达',retrying:'重试中',failed:'失败',test:'测试推送','SMS read failed':'短信读取失败','retry window expired':'已超过 3 分钟重试上限','queue capacity exceeded':'转发队列已满','network or TLS request failed':'网络连接或证书验证失败','enable at least one channel':'请至少启用一个渠道','invalid ServerChan Turbo SendKey':'Server酱 Turbo SendKey 无效','invalid webhook URL':'Webhook URL 无效','webhook does not match selected platform':'Webhook 地址与平台不匹配','webhook token missing':'Webhook 地址缺少令牌','invalid device name':'设备名称无效','wait before testing again':'请稍后再次测试'};
  root.Pages.forwarding = function () {
    return {
      language: root.Lang.getCurrentLanguage(), enabled:false,sms_enabled:true,delete_after_day:false,device_name:'',channels:[],selected:'serverchan',
      loaded:false,busy:false,dirty:false,testing:'',message:'',failed:false,error:'',queued:0,ready:false,records:[],loading:false,timer:null,
      t(key) {
        if (this.language==='zh-CN') return chinese[key] || key;
        const index=['en','ru','ar'].indexOf(this.language);
        return (words[key] && words[key][index]) || root.Lang.t(key);
      },
      platform(key) {return ({serverchan:'Server酱 Turbo',wecom:'企业微信',dingtalk:'钉钉',feishu:'飞书',webhook:'Webhook'})[key] && (this.language==='zh-CN' ? ({serverchan:'Server酱 Turbo',wecom:'企业微信',dingtalk:'钉钉',feishu:'飞书',webhook:'Webhook'})[key] : ({serverchan:'ServerChan Turbo',wecom:'WeCom',dingtalk:'DingTalk',feishu:'Feishu',webhook:'Webhook'})[key]) || key;},
      time(value) {return new Intl.DateTimeFormat(this.language,{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',second:'2-digit'}).format(new Date(value));},
      changed() {for(const c of this.channels) {if(c.clear && (c.url || c.token || c.secret)) c.clear=false;}this.dirty=true;this.message='';},
      async request(path,data) {
        const response=await root.Api.request(path,data ? {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(data)} : {});
        const result=await response.json();
        if (!response.ok || result.ok===false) throw new Error(result.error || 'Request failed');
        return result;
      },
      apply(data,form) {
        this.queued=data.queued;this.ready=data.ready;this.records=data.records;this.error=data.error;
        this.sms_enabled=data.sms_enabled;this.delete_after_day=data.delete_after_day;
        if (form) {this.enabled=data.enabled;this.device_name=data.device_name;this.channels=data.channels.map(c=>({...c,url:'',token:'',secret:'',clear:false}));this.dirty=false;this.loaded=true;}
      },
      async refresh() {
        if (this.loading || this.busy) return;
        this.loading=true;
        try {this.apply(await this.request('/api/forwarding'),!this.loaded);} catch (error) {this.message=error.message;this.failed=true;} finally {this.loading=false;}
      },
      async save() {
        this.busy=true;this.message='';
        try {
          const channels=this.channels.map(({platform,enabled,url,token,secret,clear})=>({platform,enabled,url:url.trim(),token:token.trim(),secret:secret.trim(),clear}));
          this.apply(await this.request('/api/forwarding/save',{enabled:this.enabled,sms_enabled:this.sms_enabled,delete_after_day:this.delete_after_day,device_name:this.device_name,channels}),true);
          this.message='已保存';this.failed=false;
        } catch (error) {this.message=error.message;this.failed=true;} finally {this.busy=false;}
      },
      async test(platform) {
        if (this.dirty || this.testing) return;
        this.testing=platform;this.message='';
        try {await this.request('/api/forwarding/test',{platform});this.message='测试成功';this.failed=false;} catch (error) {this.message=error.message;this.failed=true;} finally {this.testing='';await this.refresh();}
      },
      clear(c) {c.clear=true;c.enabled=false;c.url='';c.token='';c.secret='';c.has_url=false;c.has_token=false;c.has_secret=false;this.changed();},
      init() {
        this.refresh();
        const start=()=>{if (!this.timer) this.timer=setInterval(()=>this.refresh(),5000);};
        global.addEventListener('simpleadmin:page-changed',event=>{if (event.detail.page==='forwarding') {this.refresh();start();} else {clearInterval(this.timer);this.timer=null;}});
        global.addEventListener('simpleadmin:language-changed',()=>{this.language=root.Lang.getCurrentLanguage();});
        start();
      }
    };
  };
})(window);
