# AngularJS LSP 非対応機能一覧

調査テスト (`tests/investigate_unsupported_test.rs`) により判明した、現在のLSPが対応していない構文・機能の一覧。

---

## JS側（10項目）

### 1. `module.decorator()` がシンボルとして認識されない

`angular.module('app').decorator('$log', ...)` パターンが未対応。

```javascript
angular.module('app', []).decorator('$log', ['$delegate', function($delegate) {
    return $delegate;
}]);
// $log がシンボルとして登録されない
```

**対応案**: SymbolKind に Decorator 種別を追加し、`module.decorator()` 呼び出しを解析対象に含める。

---

### 2. `$resource` カスタムアクションのメソッドが抽出されない

`$resource(url, params, actions)` の第3引数で定義されるカスタムアクションが Method として認識されない。

```javascript
angular.module('app', []).factory('UserResource', ['$resource', function($resource) {
    return $resource('/api/users/:id', { id: '@id' }, {
        update: { method: 'PUT' },
        query: { method: 'GET', isArray: true }
    });
}]);
// UserResource.update, UserResource.query が Method として認識されない
```

**対応案**: factory 内の `$resource()` 呼び出しを検出し、第3引数オブジェクトのキーを Method として登録する。

---

### ~~3. factory内 `var service = {}; service.xxx` パターンが認識されない~~ (対応済み)

~~`return { ... }` 形式は認識されるが、変数にオブジェクトを代入してプロパティを追加するパターンは未対応。~~

**対応済み**: `return` される変数名を `find_returned_variable_name()` で検出し、その変数へのプロパティ代入を `extract_returned_var_method()` で Method として登録するようにした。DI配列記法・直接関数渡しの両方に対応。

---

### 4. `$stateProvider.state()` (ui-router) のコントローラー参照が追跡されない

`$routeProvider.when()` は対応済みだが、ui-router の `$stateProvider.state()` は未対応。

```javascript
angular.module('app', []).config(['$stateProvider', function($stateProvider) {
    $stateProvider
        .state('home', {
            url: '/home',
            templateUrl: 'views/home.html',
            controller: 'HomeController'   // 参照として追跡されない
        });
}]);
```

**対応案**: `$stateProvider.state()` の第2引数オブジェクトから `controller` と `templateUrl` を抽出し、テンプレートバインディングとして登録する。

---

### 5. `$scope` のネストされたプロパティ代入が追跡されない

第1レベル（`$scope.user`）のみ追跡され、2階層以上（`$scope.user.name`）は未対応。

```javascript
$scope.user = {};
$scope.user.name = 'test';      // NestedCtrl.$scope.user.name として認識されない
$scope.user.email = 'a@b.com';  // 同様
```

**対応案**: `$scope.x.y = ...` 形式の代入文を解析し、ネストされたプロパティも ScopeProperty として登録する。

---

### 6. `that = this` エイリアスが認識されない

`vm = this` と `self = this` は対応済みだが、`that = this` は未対応。

```javascript
angular.module('app', []).service('ThatSvc', [function() {
    var that = this;
    that.doWork = function() {};  // 認識されない
}]);
```

**対応案**: `src/analyzer/js/scope.rs` 付近の this エイリアス検出ロジックに `"that"` を追加する。

---

### 7. component controller内の `ctrl = this` プロパティが追跡されない

コンポーネントの controller 関数内でのプロパティ定義やライフサイクルフックが未対応。

```javascript
angular.module('app', []).component('lcComp', {
    template: '<div></div>',
    controller: function() {
        var ctrl = this;
        ctrl.data = [];                        // lcComp.data として認識されない
        ctrl.$onInit = function() {};          // ライフサイクルフック認識されない
        ctrl.$onDestroy = function() {};
        ctrl.$onChanges = function(changes) {};
    }
});
```

**対応案**: component の controller 関数内での `this` / エイリアスへのプロパティ代入を、コンポーネント名をプレフィックスとした Method として登録する。

---

### 8. provider の `$get` return オブジェクトメソッドが抽出されない

provider 自体は認識されるが、`$get` が返すオブジェクトのメソッドは未対応。

```javascript
angular.module('app', []).provider('apiProvider', function() {
    this.setUrl = function(url) {};
    this.$get = ['$http', function($http) {
        return {
            request: function(path) {},  // apiProvider.request として認識されない
            post: function(path, data) {}
        };
    }];
});
```

**対応案**: provider 内の `this.$get` を検出し、その return オブジェクトのメソッドを抽出する。

---

### 9. constant/value のオブジェクトプロパティが追跡されない

constant/value 自体は認識されるが、オブジェクト内部のプロパティは未対応。

```javascript
angular.module('app', []).constant('CONFIG', {
    API_URL: 'https://api.example.com',  // CONFIG.API_URL として認識されない
    MAX_RETRIES: 3
});
```

**対応案**: constant/value の引数がオブジェクトリテラルの場合、そのキーを Method/Property として登録する。

---

### 10. `angular.extend($scope, {...})` のプロパティが認識されない

`angular.extend` や `angular.merge` で `$scope` に一括追加されるプロパティは未対応。

```javascript
angular.extend($scope, {
    extProp1: 'hello',       // $scope.extProp1 として認識されない
    extMethod: function() {} // 同様
});
```

**対応案**: `angular.extend($scope, obj)` / `angular.merge($scope, obj)` パターンを検出し、第2引数オブジェクトのキーを ScopeProperty/ScopeMethod として登録する。

---

## HTML側（4項目）

### 11. ng-repeat 特殊変数（$index, $first, $last, $odd, $even）が認識されない

ng-repeat 内で暗黙的に利用可能な特殊変数がローカル変数/スコープ参照として登録されない。

```html
<div ng-repeat="item in items">
    {{ $index }}: {{ item.name }}         <!-- $index 未認識 -->
    <span ng-show="$first">First!</span>  <!-- $first 未認識 -->
    <span ng-show="$last">Last!</span>    <!-- $last 未認識 -->
    <span ng-class="{ 'odd': $odd }">row</span>  <!-- $odd 未認識 -->
</div>
```

**対応案**: ng-repeat 解析時に `$index`, `$first`, `$last`, `$middle`, `$odd`, `$even` をローカル変数として自動登録する。

---

### 12. ng-repeat `as` エイリアス変数が認識されない

`ng-repeat="item in items | filter:query as filtered"` の `filtered` がローカル変数として登録されない。

```html
<div ng-repeat="item in items | filter:query as filtered">
    {{ filtered.length }} items found   <!-- filtered 未認識 -->
</div>
```

**対応案**: ng-repeat 式の `as` キーワード以降をパースし、エイリアス名をローカル変数として登録する。

---

### 13. フィルター式でのネストされたプロパティ参照が認識されない

`{{ user.name | capitalize }}` のようにフィルターパイプの前のネストされたプロパティが参照として認識されない。

```html
<p>{{ user.name | capitalize }}</p>  <!-- user.name が参照として認識されない -->
```

**対応案**: フィルター式のパイプ `|` 前の式を解析する際に、ドット区切りのプロパティパスも参照として登録する。

---

### 14. ディレクティブ/コンポーネントの `=` / `&` バインディング属性値がスコープ参照として認識されない

`@` バインディングの `{{ }}` 内は認識されるが、`=`（双方向バインディング）と `&`（コールバック）の属性値はスコープ参照として登録されない。

```html
<my-widget data="widgetData" on-update="handleUpdate()"></my-widget>
<!-- data="widgetData" の widgetData がスコープ参照として認識されない -->
<!-- on-update="handleUpdate()" の handleUpdate も同様 -->

<my-dir data="myData" on-change="myChange()"></my-dir>
<!-- data="myData" の myData が認識されない -->
<!-- on-change="myChange()" の myChange も同様 -->
```

**対応案**: ディレクティブ/コンポーネントの `scope` / `bindings` 定義と照合し、`=` / `&` バインディングの属性値をスコープ参照として解析する。
